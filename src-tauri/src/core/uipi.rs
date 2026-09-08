// ============================================================================
// core/uipi.rs — Windows UIPI (User Interface Privilege Isolation) relax.
//
// PROBLEM
//   When FlexClash runs elevated (TUN mode requires admin, and we are often
//   launched from an elevated PowerShell during dev), Windows' UIPI blocks
//   drag-and-drop from a non-elevated source — i.e. a normal Explorer
//   window can no longer drop files onto us. The user gets the 🚫 cursor
//   and `tauri://drag-drop` never fires.
//
// FIX
//   `ChangeWindowMessageFilterEx(hwnd, msg, MSGFLT_ALLOW, None)` registers
//   an explicit allow-list entry for the given window handle. We relax the
//   three message IDs that participate in OS-level drag-and-drop / OLE
//   data transfer:
//
//     WM_DROPFILES      0x0233  — classic Win32 drag-drop
//     WM_COPYDATA       0x004A  — generic 32-bit data send
//     WM_COPYGLOBALDATA 0x0049  — used by the shell for global data
//                                  transfer (the modern Explorer drag path)
//
//   Doing this on the TOP-LEVEL HWND of the Tauri webview window is
//   sufficient — WebView2's child window inherits the filter, and the
//   Tauri runtime already re-broadcasts the resulting `WM_DROPFILES`
//   as `tauri://drag-drop` for the JS layer.
//
// SAFETY
//   `ChangeWindowMessageFilterEx` is a per-thread, per-HWND call. We
//   invoke it on the main UI thread during `setup`, once per window.
//   Failing the call is non-fatal — Tauri continues to start, drag/drop
//   just may be restricted in elevated windows.
// ============================================================================

#[cfg(target_os = "windows")]
pub fn relax_drag_drop_for_window(hwnd: isize) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        ChangeWindowMessageFilterEx, MSGFLT_ALLOW,
    };

    // Canonical Win32 message IDs.
    const WM_DROPFILES: u32 = 0x0233;
    const WM_COPYDATA: u32 = 0x004A;
    const WM_COPYGLOBALDATA: u32 = 0x0049;

    let handle = HWND(hwnd as *mut _);

    // We swallow the result of every `ChangeWindowMessageFilterEx` call.
    // It can fail on the very first invocation with `ERROR_ACCESS_DENIED`
    // for older builds of Windows 10 if the process was just elevated,
    // and the second call (issued by the OS on `WM_DROPFILES`) is what
    // really matters. Logging a warning is enough — the user will see
    // it in the terminal and can decide whether to investigate.
    unsafe {
        for msg in [WM_DROPFILES, WM_COPYDATA, WM_COPYGLOBALDATA] {
            if let Err(e) = ChangeWindowMessageFilterEx(handle, msg, MSGFLT_ALLOW, None) {
                eprintln!(
                    "[uipi] ChangeWindowMessageFilterEx(hwnd={hwnd:#x}, msg=0x{msg:04X}) -> {e}"
                );
            }
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn relax_drag_drop_for_window(_hwnd: isize) {
    // No-op on non-Windows platforms.
}
