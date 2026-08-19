import { describe, it, expect } from 'vitest';
import { windowsAnimatedSize } from './windowSizing';

describe('windowsAnimatedSize', () => {
  it('returns the compact fixed size', () => {
    expect(windowsAnimatedSize('compact', false)).toEqual({ width: 360, height: 380 });
  });

  it('returns the expanded fixed size when not fullscreen', () => {
    expect(windowsAnimatedSize('expanded', false)).toEqual({ width: 960, height: 640 });
  });

  it('returns a 100% size when expanded and fullscreen', () => {
    expect(windowsAnimatedSize('expanded', true)).toEqual({ width: '100%', height: '100%' });
  });

  it('ignores fullscreen when compact (fullscreen is only reachable from expanded)', () => {
    expect(windowsAnimatedSize('compact', true)).toEqual({ width: 360, height: 380 });
  });
});
