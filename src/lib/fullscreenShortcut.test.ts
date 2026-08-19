import { describe, it, expect } from 'vitest';
import { isFullscreenShortcut } from './fullscreenShortcut';

function key(over: Partial<{ key: string; metaKey: boolean; ctrlKey: boolean }>) {
  return { key: '', metaKey: false, ctrlKey: false, ...over };
}

describe('isFullscreenShortcut', () => {
  it('matches F11 alone', () => {
    expect(isFullscreenShortcut(key({ key: 'F11' }))).toBe(true);
  });

  it('matches Cmd+Ctrl+F on macOS', () => {
    expect(isFullscreenShortcut(key({ key: 'f', metaKey: true, ctrlKey: true }))).toBe(true);
  });

  it('is case-insensitive for the F key', () => {
    expect(isFullscreenShortcut(key({ key: 'F', metaKey: true, ctrlKey: true }))).toBe(true);
  });

  it('does not match plain Ctrl+F (a common find shortcut)', () => {
    expect(isFullscreenShortcut(key({ key: 'f', ctrlKey: true }))).toBe(false);
  });

  it('does not match plain Cmd+F', () => {
    expect(isFullscreenShortcut(key({ key: 'f', metaKey: true }))).toBe(false);
  });

  it('does not match an unrelated key', () => {
    expect(isFullscreenShortcut(key({ key: 'a', metaKey: true, ctrlKey: true }))).toBe(false);
  });
});
