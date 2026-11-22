"use client";

import { formatCost } from "../../list/AgentTracesTable";
import { SpanIcon } from "../../ui/SpanIcon";
import type { TurnRowData } from "../../tree";
import type { Span, Trace } from "../../types";
import { fmtMs, fmtTok } from "../../utils";
import { useSpanDetail } from "../content/ContentTab";
import { Payload } from "../content/PayloadBody";
import { payloadView, toolSummary } from "../content/payload";
import { turnFacts } from "../tree/SpanHoverCard";
import { PaneHeader } from "./PaneHeader";

type Fact = readonly [label: string, value: string];

const HEADING = "text-xs font-medium text-muted-foreground";

const paneFacts = (row: TurnRowData): readonly Fact[] => {
  const facts = turnFacts(row);
  const optional: readonly (Fact | null)[] = [
    facts.tokens > 0 ? ["Tokens", fmtTok(facts.tokens)] : null,
    facts.spend != null ? ["Cost", formatCost(facts.spend)] : null,
    ["Model calls", String(facts.modelCalls ?? 0)],
    ["Tool calls", String(facts.toolCalls ?? 0)],
  ];
  return [["Duration", fmtMs(facts.durationMs)], ...optional.filter((fact): fact is Fact => fact !== null)];
};

type Placed = { span: Span; depth: number };

const under = (members: readonly Span[], parent: string | null, depth: number): Placed[] =>
  members
    .filter((span) => span.parent_span_id === parent)
    .flatMap((span) =>
      span.type === "framework"
        ? under(members, span.span_id, depth)
        : [{ span, depth }, ...under(members, span.span_id, depth + 1)],
    );

const REPLY = "claude_code.assistant_response";

function Step({ span, depth, onOpen }: Placed & { onOpen: (id: string) => void }) {
  const failed = span.status === "error";
  const hint = span.type === "tool" ? toolSummary(span.input_preview) : "";
  return (
    <li>
      <button
        type="button"
        onClick={() => onOpen(span.span_id)}
        className="flex h-8 w-full min-w-0 items-center gap-2 rounded-md px-2 text-left hover:bg-muted/60 focus-visible:shadow-[inset_0_0_0_2px_var(--ring)] focus-visible:outline-none"
        style={{ paddingLeft: 8 + depth * 16 }}
      >
        <SpanIcon type={span.type} model={span.model} error={failed} size="md" />
        <span className={failed ? "shrink-0 font-mono text-xs text-destructive" : "shrink-0 font-mono text-xs"}>
          {span.type === "llm" ? span.model || span.name : span.name}
        </span>
        <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">{hint}</span>
        <span className="shrink-0 font-mono text-[11px] text-muted-foreground tabular-nums">
          {fmtMs(span.duration_ms)}
        </span>
      </button>
    </li>
  );
}

/** One typed message: what the user asked, what the agent answered, and the steps in between, all as Claude Code recorded them. */
export function TurnPane({
  trace,
  row,
  accessToken,
  onOpenStep,
  onClose,
}: {
  trace: Trace;
  row: TurnRowData;
  accessToken: string;
  onOpenStep: (id: string) => void;
  onClose: () => void;
}) {
  const facts = turnFacts(row);
  const message = row.root.input_preview;
  const replied = row.members.findLast((span) => span.name === REPLY && span.parent_span_id === row.root.span_id);
  const answer = useSpanDetail(
    accessToken,
    trace.summary.trace_id,
    replied?.span_id ?? null,
    trace.summary.trace_ref,
  ).data;
  return (
    <aside className="flex h-full min-w-0 flex-col bg-background text-sm text-foreground" aria-label="Turn details">
      <PaneHeader
        type="agent"
        model={null}
        failed={facts.failed}
        title={facts.name}
        facts={paneFacts(row)}
        onClose={onClose}
      />
      <div className="flex min-h-0 flex-1 flex-col gap-5 overflow-auto px-4 py-4">
        {message && (
          <section aria-label="User message" className="flex flex-col gap-1.5">
            <h3 className={HEADING}>User</h3>
            <p className="rounded-lg border border-border bg-card px-3 py-2.5 text-sm whitespace-pre-wrap text-foreground">
              {message}
            </p>
          </section>
        )}
        {replied && answer?.output && (
          <section aria-label="Agent answer" className="flex flex-col gap-1.5">
            <h3 className={HEADING}>Agent</h3>
            <Payload
              view={payloadView(answer.output, answer.output_ui, false, true)}
              name={row.root.name}
              failed={row.root.status === "error"}
            />
          </section>
        )}
        <section aria-label="Turn steps" className="flex flex-col gap-1.5">
          <h3 className={HEADING}>Steps</h3>
          <ol className="flex flex-col">
            {under(row.members, row.root.span_id, 0).map(({ span, depth }) => (
              <Step key={span.span_id} span={span} depth={depth} onOpen={onOpenStep} />
            ))}
          </ol>
        </section>
      </div>
    </aside>
  );
}
