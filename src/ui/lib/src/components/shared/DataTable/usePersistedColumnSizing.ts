import type { ColumnSizingState, OnChangeFn } from "@tanstack/react-table";

import { usePersistedJson } from "./usePersistedJson";

const STORAGE_KEY_PREFIX = "litellm_table_column_widths_";

const EMPTY_SIZING: ColumnSizingState = {};

function isColumnSizing(value: unknown): value is ColumnSizingState {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
  return Object.values(value).every((width) => typeof width === "number" && Number.isFinite(width) && width > 0);
}

const parseSizing = (stored: unknown): ColumnSizingState => (isColumnSizing(stored) ? stored : EMPTY_SIZING);

export function usePersistedColumnSizing(tableId: string): {
  columnSizing: ColumnSizingState;
  onColumnSizingChange: OnChangeFn<ColumnSizingState>;
} {
  const [columnSizing, onColumnSizingChange] = usePersistedJson(`${STORAGE_KEY_PREFIX}${tableId}`, parseSizing);
  return { columnSizing, onColumnSizingChange };
}
