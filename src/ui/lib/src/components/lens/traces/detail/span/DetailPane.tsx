"use client";

import type { TreeRow } from "../../tree";
import type { Trace } from "../../types";
import { GroupPane } from "./GroupPane";
import { TurnPane } from "./TurnPane";
import { SpanPane, type SpanTabProps } from "./SpanPane";

interface DetailPaneProps extends SpanTabProps {
  trace: Trace;
  row: TreeRow | undefined;
  accessToken: string;
  onOpenStep: (id: string) => void;
  onClose: () => void;
}

/** Right pane of the run view: switches on the selected tree row. */
export function DetailPane({
  trace,
  row,
  accessToken,
  spanTab,
  onSpanTabChange,
  onOpenStep,
  onClose,
}: DetailPaneProps) {
  if (!row || row.kind === "load-more") {
    return (
      <div className="grid h-full place-items-center bg-background text-sm text-muted-foreground">
        Select a span to inspect it.
      </div>
    );
  }
  if (row.kind === "turn")
    return (
      <TurnPane
        key={row.id}
        trace={trace}
        row={row}
        accessToken={accessToken}
        onOpenStep={onOpenStep}
        onClose={onClose}
      />
    );
  if (row.kind === "group")
    return <GroupPane key={row.id} trace={trace} row={row} accessToken={accessToken} onClose={onClose} />;
  return (
    <SpanPane
      key={row.id}
      trace={trace}
      span={row.span}
      accessToken={accessToken}
      spanTab={spanTab}
      onSpanTabChange={onSpanTabChange}
      onClose={onClose}
    />
  );
}
