"use client";

import { useMemo } from "react";
import {
  ArrowUpRight,
  Check,
  CircleCheck,
  GitMerge,
  GitPullRequest,
  MessageSquare,
  Workflow,
} from "lucide-react";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import type { TraceSummary } from "../traces/types";
import { useOutcomesRoute } from "../route";
import { outcomeForTrace } from "./outcomeTrace";

export function TraceDeliveryOutcome({ summary, live }: { summary: TraceSummary; live: boolean }) {
  const route = useOutcomesRoute();
  const session = useMemo(
    () => (live ? undefined : outcomeForTrace(summary.trace_id, Date.now(), summary.start_time)),
    [live, summary.trace_id, summary.start_time],
  );
  if (!session) return null;
  const pr = session.pullRequest;
  const issue = session.issue;
  const stages = [
    { label: "Prompt", value: session.title, detail: "Task recorded", icon: MessageSquare, done: true },
    {
      label: "Execution",
      value: `${summary.span_count} recorded steps`,
      detail: `${summary.llm_calls} model call · $${session.costUSD.toFixed(2)}`,
      icon: Workflow,
      done: true,
    },
    {
      label: "Pull request",
      value: pr ? `PR #${pr.id} ${pr.status === "merged" ? "merged" : pr.status}` : "No PR linked",
      detail: pr ? session.repository : "Outcome unknown",
      icon: pr?.status === "merged" ? GitMerge : GitPullRequest,
      done: pr?.status === "merged",
    },
    {
      label: "Issue",
      value: issue ? `Issue #${issue.id} ${issue.status}` : "No issue linked",
      detail: issue ? session.repository : "Outcome unknown",
      icon: CircleCheck,
      done: issue?.status === "closed",
    },
  ];
  return (
    <section aria-label="Delivery outcomes" className="shrink-0 border-b bg-card px-4 py-3">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <h3 className="text-xs font-medium">Trace to delivery</h3>
          <Badge variant="outline" className="h-4 text-[10px]">
            Sample data
          </Badge>
        </div>
        <Button variant="ghost" size="xs" className="h-6 text-[11px] text-info" onClick={route.showDashboard}>
          Aggregate outcomes <ArrowUpRight className="size-3" />
        </Button>
      </div>
      <ol className="grid grid-cols-2 divide-x border-y bg-muted/10 lg:grid-cols-4">
        {stages.map(({ label, value, detail, icon: Icon, done }) => (
          <li key={label} className="min-w-0 px-3 py-2">
            <div className="flex items-center justify-between gap-2">
              <p className="flex items-center gap-1.5 text-[10px] text-muted-foreground">
                <Icon aria-hidden="true" className="size-3" />
                {label}
              </p>
              {done && <Check aria-hidden="true" className="size-3 text-chart-2" />}
            </div>
            <p
              className={`mt-1 truncate text-xs font-medium ${done && (label === "Pull request" || label === "Issue") ? "text-chart-2" : ""}`}
              title={value}
            >
              {value}
            </p>
            <p className="mt-1 truncate font-mono text-[9px] text-muted-foreground">{detail}</p>
          </li>
        ))}
      </ol>
      <p className="mt-2 text-[10px] text-muted-foreground">
        Explicit repository and artifact IDs link this trace to sample GitHub merge and close events received
        after execution.
      </p>
    </section>
  );
}
