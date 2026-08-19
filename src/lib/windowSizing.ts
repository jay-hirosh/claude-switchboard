/** Target size for the Windows-only DOM-animated container in App.tsx (see
 *  its comment: rapid native resizing is laggy on Windows, so the DOM
 *  animates instead and the OS window is sized to match). Expanded
 *  fullscreen fills the window instead of the fixed 960x640 — the OS window
 *  itself is already fullscreen-sized by that point, so a fixed pixel size
 *  here would letterbox the content inside it. */
export function windowsAnimatedSize(
  viewMode: 'compact' | 'expanded',
  isFullscreen: boolean,
): { width: number | string; height: number | string } {
  if (viewMode === 'expanded' && isFullscreen) return { width: '100%', height: '100%' };
  return viewMode === 'expanded' ? { width: 960, height: 640 } : { width: 360, height: 380 };
}
