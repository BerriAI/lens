export interface DemoStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}
export const DEMO_STORAGE_PREFIX = "lens-synthetic-demo-v2:";
export function browserDemoStorage(): DemoStorage | undefined {
  try {
    return typeof window === "undefined" ? undefined : window.localStorage;
  } catch {
    return undefined;
  }
}
export function readDemoState(storage: DemoStorage | undefined, key: string): unknown {
  try {
    return JSON.parse(storage?.getItem(DEMO_STORAGE_PREFIX + key) ?? "null");
  } catch {
    return null;
  }
}
export function writeDemoState(storage: DemoStorage | undefined, key: string, value: unknown) {
  try {
    storage?.setItem(DEMO_STORAGE_PREFIX + key, JSON.stringify(value));
  } catch {
    /* Storage may be disabled. Keep this session usable. */
  }
}
export function resetDemoStorage() {
  const storage = browserDemoStorage();
  for (const key of ["optimizations", "datasets", "evals"]) storage?.removeItem(DEMO_STORAGE_PREFIX + key);
}
