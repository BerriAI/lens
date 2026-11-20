import type { OnChangeFn, VisibilityState } from "@tanstack/react-table";
import { useCallback } from "react";

import { usePersistedJson } from "./usePersistedJson";

const STORAGE_KEY_PREFIX = "litellm_table_columns_";

const EMPTY_VISIBILITY: VisibilityState = {};

function isVisibilityState(value: unknown): value is VisibilityState {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }
  return Object.values(value).every((visible) => typeof visible === "boolean");
}

export function usePersistedColumnVisibility(
  tableId: string,
  defaults: VisibilityState = EMPTY_VISIBILITY,
): { columnVisibility: VisibilityState; onColumnVisibilityChange: OnChangeFn<VisibilityState> } {
  const parse = useCallback(
    (stored: unknown): VisibilityState => (isVisibilityState(stored) ? { ...defaults, ...stored } : defaults),
    [defaults],
  );
  const [columnVisibility, onColumnVisibilityChange] = usePersistedJson(`${STORAGE_KEY_PREFIX}${tableId}`, parse);
  return { columnVisibility, onColumnVisibilityChange };
}
