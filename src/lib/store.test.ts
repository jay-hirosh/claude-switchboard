import { describe, it, expect, beforeEach, vi } from 'vitest';

// vi.hoisted() runs before the vi.mock() factory, making the mock reference
// available to the hoisted factory without a top-level variable problem.
const { listenMock } = vi.hoisted(() => ({
  listenMock: vi.fn(),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: listenMock,
}));

// Stub the Tauri window API so getCurrentWindow() doesn't throw.
// onFocusChanged must return a spy (not undefined) because store.ts
// now captures and stores the returned unlistener.
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    onFocusChanged: vi.fn().mockResolvedValue(vi.fn()),
  }),
}));

// Stub ipc so init()'s Promise.all resolves quickly with minimal data.
vi.mock('./ipc', () => ({
  ipc: {
    getCurrentUsage: vi.fn().mockResolvedValue(null),
    getSettings: vi.fn().mockResolvedValue(null),
    hasClaudeCodeCreds: vi.fn().mockResolvedValue(false),
    listAccounts: vi.fn().mockResolvedValue([]),
    getLiveSessions: vi.fn().mockResolvedValue([]),
    toggleFullscreen: vi.fn().mockResolvedValue(true),
  },
}));

// Import store AFTER mocks are set up.
import { useAppStore } from './store';
import { ipc } from './ipc';

describe('useAppStore — event-listener lifecycle', () => {
  beforeEach(() => {
    // Reset mock call history and make each listen() call return a unique
    // unlistener spy so we can assert per-listener invocation counts.
    listenMock.mockReset();
    listenMock.mockImplementation(() => Promise.resolve(vi.fn()));

    // Clear any module-level _unlisteners left by a previous test.
    useAppStore.getState().cleanup();
  });

  it('calls each first-init unlistener exactly once when init() is called a second time', async () => {
    // First init — registers 7 listeners (one per event type in subscribe()).
    await useAppStore.getState().init();

    // Collect the resolved unlistener spies from the first init.
    const firstUnlisteners = await Promise.all(
      listenMock.mock.results.map((r) => r.value as Promise<ReturnType<typeof vi.fn>>),
    );

    // Second init — must tear down the first set before registering new ones.
    await useAppStore.getState().init();

    // Every unlistener from the first init must have been invoked exactly once.
    for (const unlisten of firstUnlisteners) {
      expect(unlisten).toHaveBeenCalledTimes(1);
    }
  });

  it('cleanup() removes all listeners and is idempotent', async () => {
    await useAppStore.getState().init();

    const unlisteners = await Promise.all(
      listenMock.mock.results.map((r) => r.value as Promise<ReturnType<typeof vi.fn>>),
    );

    useAppStore.getState().cleanup();

    for (const unlisten of unlisteners) {
      expect(unlisten).toHaveBeenCalledTimes(1);
    }

    // Calling cleanup() again must not double-invoke the unlisteners.
    useAppStore.getState().cleanup();
    for (const unlisten of unlisteners) {
      expect(unlisten).toHaveBeenCalledTimes(1);
    }
  });

  it('second-init unlisteners are cleaned up by a third init', async () => {
    // First init.
    await useAppStore.getState().init();
    const firstResults = [...listenMock.mock.results];
    listenMock.mockClear();

    // Second init tears down first batch.
    await useAppStore.getState().init();
    const secondResults = [...listenMock.mock.results];
    listenMock.mockClear();

    // Third init tears down second batch.
    await useAppStore.getState().init();

    // First-batch unlisteners: each called exactly once (during second init).
    const firstUnlisteners = await Promise.all(
      firstResults.map((r) => r.value as Promise<ReturnType<typeof vi.fn>>),
    );
    for (const unlisten of firstUnlisteners) {
      expect(unlisten).toHaveBeenCalledTimes(1);
    }

    // Second-batch unlisteners: each called exactly once (during third init).
    const secondUnlisteners = await Promise.all(
      secondResults.map((r) => r.value as Promise<ReturnType<typeof vi.fn>>),
    );
    for (const unlisten of secondUnlisteners) {
      expect(unlisten).toHaveBeenCalledTimes(1);
    }
  });
});

describe('useAppStore — fullscreen', () => {
  beforeEach(() => {
    vi.mocked(ipc.toggleFullscreen).mockReset();
    useAppStore.setState({ isFullscreen: false });
  });

  it('defaults to isFullscreen: false', () => {
    expect(useAppStore.getState().isFullscreen).toBe(false);
  });

  it('toggleFullscreen() calls ipc.toggleFullscreen() and adopts its resolved state', async () => {
    vi.mocked(ipc.toggleFullscreen).mockResolvedValue(true);

    await useAppStore.getState().toggleFullscreen();

    expect(ipc.toggleFullscreen).toHaveBeenCalledTimes(1);
    expect(useAppStore.getState().isFullscreen).toBe(true);
  });

  it('setFullscreen() sets isFullscreen without calling ipc', () => {
    useAppStore.getState().setFullscreen(true);

    expect(useAppStore.getState().isFullscreen).toBe(true);
    expect(ipc.toggleFullscreen).not.toHaveBeenCalled();
  });
});
