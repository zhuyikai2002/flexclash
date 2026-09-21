// ============================================================================
// utils/platform.ts — Renderer-side OS detection.
//
// FlexClash ships a `@tauri-apps/plugin-os`-free platform probe so the UI
// can switch Windows/Linux copy without adding a plugin dependency. Inside
// the Tauri webview `navigator.userAgent` and `navigator.platform` are
// reliable enough for the two-branch UI decisions in this app (autostart
// path hint, TUN adapter label).
// ============================================================================

export type DesktopPlatform = 'windows' | 'linux' | 'macos' | 'unknown'

/** Detect the current desktop OS from the browser/webview environment. */
export function detectPlatform(): DesktopPlatform {
  if (typeof navigator === 'undefined') return 'unknown'

  const ua = navigator.userAgent ?? ''
  const platform = navigator.platform ?? ''

  // Order matters: "Macintosh" appears in iPad user agents, and Android
  // user agents contain "Linux". On the desktop webviews FlexClash targets
  // this ordering is stable.
  if (/Windows/i.test(ua) || /Win/i.test(platform)) return 'windows'
  if (/Macintosh|Mac OS X/i.test(ua) || /Mac/i.test(platform)) return 'macos'
  if (/Linux/i.test(ua) || /Linux/i.test(platform)) return 'linux'

  return 'unknown'
}

/** True when running on a Linux desktop (GNOME/Wayland, KDE, etc.). */
export function isLinux(): boolean {
  return detectPlatform() === 'linux'
}
