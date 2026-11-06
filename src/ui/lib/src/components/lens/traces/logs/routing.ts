import { parseAsBoolean, parseAsString, useQueryStates } from "nuqs";
import { useCallback } from "react";

export const LOG_TIMELINE_PARSERS = {
  logs: parseAsBoolean.withDefault(false),
  log_cursor: parseAsString,
  log_record: parseAsString,
};

export function useLogTimelineRouting() {
  const [params, setParams] = useQueryStates(LOG_TIMELINE_PARSERS, {
    history: "push",
  });
  const setOpen = useCallback(
    (open: boolean) =>
      void setParams(
        open
          ? { logs: true }
          : { logs: null, log_cursor: null, log_record: null },
      ),
    [setParams],
  );
  const setCursor = useCallback(
    (cursor: string | null) =>
      void setParams({ log_cursor: cursor, log_record: null }),
    [setParams],
  );
  const setRecordId = useCallback(
    (recordId: string | null) => void setParams({ log_record: recordId }),
    [setParams],
  );
  const reset = useCallback(
    () =>
      void setParams(
        { log_cursor: null, log_record: null },
        { history: "replace" },
      ),
    [setParams],
  );
  return {
    open: params.logs,
    cursor: params.log_cursor,
    recordId: params.log_record,
    setOpen,
    setCursor,
    setRecordId,
    reset,
  };
}
