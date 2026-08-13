import "@testing-library/jest-dom/vitest";

// Node 22+ defines its own global `localStorage`/`sessionStorage` (Web
// Storage API). It loads before vitest's jsdom environment is populated and
// wins the name — vitest's environment setup only assigns globals that
// aren't already present, so jsdom's own (working) Storage implementation
// never gets attached, and app code sees Node's version instead, which
// throws on every call without a `--localstorage-file` backing it. Replace
// both with a plain in-memory Storage so `localStorage`/`sessionStorage`
// work the same as a real browser's for the lifetime of the test run.
class MemoryStorage implements Storage {
  #data = new Map<string, string>();
  get length() {
    return this.#data.size;
  }
  clear() {
    this.#data.clear();
  }
  getItem(key: string) {
    return this.#data.get(key) ?? null;
  }
  key(index: number) {
    return [...this.#data.keys()][index] ?? null;
  }
  removeItem(key: string) {
    this.#data.delete(key);
  }
  setItem(key: string, value: string) {
    this.#data.set(key, value);
  }
}

Object.defineProperty(globalThis, "localStorage", {
  value: new MemoryStorage(),
  configurable: true,
});
Object.defineProperty(globalThis, "sessionStorage", {
  value: new MemoryStorage(),
  configurable: true,
});
