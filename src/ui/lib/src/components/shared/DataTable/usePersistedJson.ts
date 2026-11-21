import { useCallback, useMemo, useSyncExternalStore } from "react";

import {
  LOCAL_STORAGE_EVENT,
  emitLocalStorageChange,
  getLocalStorageItem,
  setLocalStorageItem,
} from "../../../utils/localStorageUtils";

const unsavedWrites = new Map<string, string>();

function forgetUnsavedWrite(event: StorageEvent): void {
  if (event.key === null) {
    unsavedWrites.clear();
    return;
  }
  unsavedWrites.delete(event.key);
}

function subscribe(onChange: () => void): () => void {
  const onStorage = (event: StorageEvent): void => {
    forgetUnsavedWrite(event);
    onChange();
  };
  window.addEventListener("storage", onStorage);
  window.addEventListener(LOCAL_STORAGE_EVENT, onChange);
  return () => {
    window.removeEventListener("storage", onStorage);
    window.removeEventListener(LOCAL_STORAGE_EVENT, onChange);
  };
}

function readRaw(key: string): string | null {
  return unsavedWrites.get(key) ?? getLocalStorageItem(key);
}

function writeRaw(key: string, raw: string): void {
  setLocalStorageItem(key, raw);
  if (getLocalStorageItem(key) === raw) {
    unsavedWrites.delete(key);
  } else {
    unsavedWrites.set(key, raw);
  }
  emitLocalStorageChange(key);
}

function parseJson(raw: string | null): unknown {
  if (raw === null) return null;
  try {
    return JSON.parse(raw);
  } catch {
    return null;
  }
}

type Updater<T> = T | ((previous: T) => T);

/** `parse` must be referentially stable: it turns the stored JSON (null when absent or malformed) into the value. */
export function usePersistedJson<T>(key: string, parse: (stored: unknown) => T): [T, (updater: Updater<T>) => void] {
  const raw = useSyncExternalStore(
    subscribe,
    () => readRaw(key),
    () => null,
  );
  const value = useMemo(() => parse(parseJson(raw)), [raw, parse]);
  const setValue = useCallback(
    (updater: Updater<T>) => {
      const next =
        typeof updater === "function" ? (updater as (previous: T) => T)(parse(parseJson(readRaw(key)))) : updater;
      writeRaw(key, JSON.stringify(next));
    },
    [key, parse],
  );
  return [value, setValue];
}
