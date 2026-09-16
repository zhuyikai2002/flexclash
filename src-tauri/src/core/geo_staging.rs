// ============================================================================
// core/geo_staging.rs — a throwaway loopback HTTP server used to hand verified
// geo databases to the kernel.
//
// WHY THIS EXISTS
// ---------------
// The kernel caches *parsed* geo matchers in permanent `singleflight` groups
// and exposes no REST entry point to clear them; only its own updater does,
// and only on the branch where it actually downloaded something (see the long
// note at the top of `core::geodata`). So the bytes we verify must be fetched
// by the kernel itself. We therefore stage them behind a loopback URL and let
// the kernel pull them from us.
//
// WHY NOT A FRAMEWORK
// -------------------
// The contract is two `GET`s of two files, served to exactly one client, for a
// few seconds, on a port that is closed again immediately afterwards. Pulling
// `axum`/`hyper`'s server half into the dependency graph for that would be
// several times the code and a permanent build-time cost for a transient need;
// `tokio` — already a dependency for the kernel ingest — carries everything
// required. What is implemented here is deliberately the *minimum* that Go's
// `net/http` client (which is what mihomo uses) will accept: complete headers,
// an accurate `Content-Length`, and `Connection: close`.
//
// SELF-DESTRUCT
// -------------
// The server never outlives the refresh. `StagingServer::shutdown` is called
// the moment the kernel's fetch returns, and `STAGING_MAX_LIFETIME` is a hard
// backstop for the paths that cannot reach that call (an early `?`, a panic
// elsewhere) so a half-finished refresh cannot leave a port bound forever.
// ============================================================================

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::time::{sleep_until, Instant};

use crate::error::AppError;

/// Upper bound on the request head we will buffer.
///
/// The kernel sends a handful of short headers; anything larger is not our
/// client and is dropped rather than allowed to grow the buffer.
const MAX_HEADER_BYTES: usize = 8 * 1024;

/// Hard backstop on how long the server may stay up. See the module note.
const STAGING_MAX_LIFETIME: Duration = Duration::from_secs(30);

/// How long `shutdown` waits for the accept loop to observe the signal.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

/// One file the kernel may fetch.
pub struct StagedFile {
    /// The path segment it is served under, i.e. `/GeoSite.dat`.
    pub name: String,
    /// The verified bytes. `Arc` so every connection hands out the same
    /// allocation instead of cloning ~21 MB per request.
    pub bytes: Arc<Vec<u8>>,
}

/// A running loopback staging server.
pub struct StagingServer {
    port: u16,
    hits: Arc<AtomicU64>,
    served_bytes: Arc<AtomicU64>,
    shutdown: Option<oneshot::Sender<()>>,
    accept_loop: tokio::task::JoinHandle<()>,
}

impl StagingServer {
    /// Bind `127.0.0.1:0` (an ephemeral port, so two refreshes can never
    /// collide on a fixed one) and start serving `files`.
    pub async fn start(files: Vec<StagedFile>) -> Result<Self, AppError> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| AppError::Geo(format!("bind staging listener: {e}")))?;
        let port = listener
            .local_addr()
            .map_err(|e| AppError::Geo(format!("staging listener addr: {e}")))?
            .port();

        let table: Arc<HashMap<String, Arc<Vec<u8>>>> = Arc::new(
            files
                .into_iter()
                .map(|f| (f.name, f.bytes))
                .collect::<HashMap<_, _>>(),
        );

        let (tx, rx) = oneshot::channel::<()>();
        let hits = Arc::new(AtomicU64::new(0));
        let served_bytes = Arc::new(AtomicU64::new(0));

        let accept_loop = tokio::spawn(accept_loop(
            listener,
            table,
            Arc::clone(&hits),
            Arc::clone(&served_bytes),
            rx,
        ));

        Ok(Self {
            port,
            hits,
            served_bytes,
            shutdown: Some(tx),
            accept_loop,
        })
    }

    /// The ephemeral port the kernel must be pointed at.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The URL to hand the kernel for one staged file.
    pub fn url_for(&self, name: &str) -> String {
        format!("http://127.0.0.1:{}/{name}", self.port)
    }

    /// How many `GET`s actually hit a staged file.
    ///
    /// This is the positive proof that the kernel really reached us — a
    /// refresh can "succeed" while fetching nothing (see the enabled-flags
    /// note in `core::geodata`), and the difference is only visible here.
    pub fn hits(&self) -> u64 {
        self.hits.load(Ordering::Relaxed)
    }

    /// Total bytes handed to the kernel.
    pub fn served_bytes(&self) -> u64 {
        self.served_bytes.load(Ordering::Relaxed)
    }

    /// Stop accepting and wait briefly for the loop to unwind. Consumes the
    /// server, so a caller cannot accidentally keep using a dead one.
    pub async fn shutdown(mut self) -> (u64, u64) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        let _ = tokio::time::timeout(SHUTDOWN_GRACE, self.accept_loop).await;
        (
            self.hits.load(Ordering::Relaxed),
            self.served_bytes.load(Ordering::Relaxed),
        )
    }
}

async fn accept_loop(
    listener: TcpListener,
    table: Arc<HashMap<String, Arc<Vec<u8>>>>,
    hits: Arc<AtomicU64>,
    served_bytes: Arc<AtomicU64>,
    mut shutdown: oneshot::Receiver<()>,
) {
    let deadline = Instant::now() + STAGING_MAX_LIFETIME;
    loop {
        tokio::select! {
            // Explicit shutdown — the normal path.
            _ = &mut shutdown => break,
            // Backstop: never hold the port past the lifetime cap.
            _ = sleep_until(deadline) => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, _peer)) => {
                    let table = Arc::clone(&table);
                    let hits = Arc::clone(&hits);
                    let served = Arc::clone(&served_bytes);
                    // One task per connection: a slow or stalled client must
                    // not block the accept loop, and every error is swallowed
                    // into a closed socket (the kernel sees a transport error
                    // and reports it; we have no channel to log to here).
                    tokio::spawn(async move {
                        let _ = serve_one(stream, table, hits, served).await;
                    });
                }
                // Transient accept errors (e.g. a client that hung up during
                // the handshake) must not tear the server down.
                Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
            },
        }
    }
}

async fn serve_one(
    mut stream: TcpStream,
    table: Arc<HashMap<String, Arc<Vec<u8>>>>,
    hits: Arc<AtomicU64>,
    served_bytes: Arc<AtomicU64>,
) -> std::io::Result<()> {
    let Some(head) = read_head(&mut stream).await else {
        return Ok(());
    };

    // `GET /GeoSite.dat HTTP/1.1`
    let mut parts = head.lines().next().unwrap_or_default().split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    // Strip any query/fragment; only the path segment names a file.
    let name = target
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .trim_start_matches('/');

    if method != "GET" && method != "HEAD" {
        return write_status(&mut stream, 405, "Method Not Allowed").await;
    }

    let Some(bytes) = table.get(name) else {
        return write_status(&mut stream, 404, "Not Found").await;
    };

    hits.fetch_add(1, Ordering::Relaxed);
    let head = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: application/octet-stream\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         Cache-Control: no-store\r\n\
         \r\n",
        bytes.len()
    );
    stream.write_all(head.as_bytes()).await?;

    if method == "GET" {
        served_bytes.fetch_add(bytes.len() as u64, Ordering::Relaxed);
        stream.write_all(bytes.as_slice()).await?;
    }
    stream.flush().await?;
    let _ = stream.shutdown().await;
    Ok(())
}

/// Read up to the end of the request head. `None` means the peer went away or
/// the head exceeded `MAX_HEADER_BYTES` — both are a silent close, not an
/// error worth propagating.
async fn read_head(stream: &mut TcpStream) -> Option<String> {
    let mut buf: Vec<u8> = Vec::with_capacity(512);
    let mut chunk = [0u8; 512];
    loop {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if buf.len() > MAX_HEADER_BYTES {
            return None;
        }
    }
    // Headers are ASCII; lossy decoding cannot fail us here.
    Some(String::from_utf8_lossy(&buf).into_owned())
}

async fn write_status(stream: &mut TcpStream, code: u16, reason: &str) -> std::io::Result<()> {
    let body = format!("{code} {reason}\n");
    let head = format!(
        "HTTP/1.1 {code} {reason}\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    stream.flush().await?;
    let _ = stream.shutdown().await;
    Ok(())
}
