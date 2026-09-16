; ----------------------------------------------------------------------------
; installer.nsh — FlexClash NSIS installer hooks.
;
; Wired in via `bundle.windows.nsis.installerHooks` in tauri.conf.json. The
; Tauri v2 NSIS template `!include`s this file and, for each hook macro we
; define, `!insertmacro`s it at the matching point of the install lifecycle
; (guarded by `!ifmacrodef`, so defining a hook is opt-in).
;
; NSIS_HOOK_PREINSTALL runs at the very start of `Section Install`, AFTER the
; template's own `CheckIfAppIsRunning` (which only force-closes the *main* app
; binary) but BEFORE every `File` extraction. That is precisely the window
; where a still-running sidecar locks its own `.exe` and makes the installer
; fail with "...mihomo... is in use" — the classic silent `/S` update failure.
;
; The sidecar's on-disk name carries the Rust target triple, so the image is
; `mihomo-x86_64-pc-windows-msvc.exe` (NOT `mihomo.exe`). The wildcard below
; matches both, mirroring `core::sidecar::hard_cleanup`.
; ----------------------------------------------------------------------------

!macro NSIS_HOOK_PREINSTALL
  ; Force-kill any zombie mihomo sidecar so the `File` extraction can overwrite
  ; it in both GUI and /S silent installs. nsExec runs it hidden and waits.
  nsExec::ExecToStack 'taskkill /F /T /IM mihomo*.exe'
  Pop $0  ; exit code (0 = killed, 128 = no such process — both fine)
  Pop $1  ; taskkill stdout/stderr text (diagnostic, unused)
  ; TerminateProcess is synchronous, but give the kernel a beat to release the
  ; file handle before the extraction loop starts.
  Sleep 300
!macroend
