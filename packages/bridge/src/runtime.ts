// The native webview injects `__TAURI_INTERNALS__`; plain HTTP access to the
// dev server does not, and IPC (invoke, Channel) is unavailable there.
export function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
}
