//! Unified application error type. Serializes to a plain string for the frontend.

use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("io error: {0}")]
    Io(String),

    #[error("path error: {0}")]
    Path(String),

    #[error("shell sidecar error: {0}")]
    Shell(String),

    #[error("config error: {0}")]
    Config(String),

    #[error("subscription error: {0}")]
    Subscription(String),

    #[error("system-proxy error: {0}")]
    Proxy(String),

    #[error("tray error: {0}")]
    Tray(String),

    #[error("desktop integration error: {0}")]
    Desktop(String),

    #[error("TUN error: {0}")]
    Tun(String),

    #[error("elevation error: {0}")]
    Elevation(String),

    #[error("storage error: {0}")]
    Storage(String),

    #[error("http error: {0}")]
    Http(String),

    #[error("kernel already running")]
    AlreadyRunning,

    #[error("kernel not running")]
    NotRunning,

    #[error("{0}")]
    Other(String),
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self { AppError::Io(e.to_string()) }
}

impl From<tauri::Error> for AppError {
    fn from(e: tauri::Error) -> Self { AppError::Other(e.to_string()) }
}

impl From<tauri_plugin_shell::Error> for AppError {
    fn from(e: tauri_plugin_shell::Error) -> Self { AppError::Shell(e.to_string()) }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self { AppError::Config(e.to_string()) }
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self { AppError::Storage(e.to_string()) }
}

pub type Result<T> = std::result::Result<T, AppError>;

// Tauri requires command return errors to be Serialize.
impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}
