/** Whether a keydown event is the "toggle fullscreen" shortcut: F11
 *  (Windows) or Cmd+Ctrl+F (macOS). Requires both modifiers on macOS so a
 *  plain Cmd+F / Ctrl+F — commonly "find" — doesn't also toggle fullscreen. */
export function isFullscreenShortcut(e: {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
}): boolean {
  if (e.key === 'F11') return true;
  return e.metaKey && e.ctrlKey && e.key.toLowerCase() === 'f';
}
