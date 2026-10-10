import { Activity, LoaderCircle } from "lucide-react";
import { useNow } from "../../../hooks/useNow";
import type { InvestigatorStatus } from "../model/types";

const labels: Record<InvestigatorStatus["state"], string> = {
  starting: "Investigator starting",
  running: "Analyzing traces",
  scheduled: "Automatic findings active",
  stalled: "Investigator overdue",
  failed: "Last check needs attention",
  daily_limit: "Daily analysis budget reached",
  unavailable: "Investigator status unavailable",
};

export function InvestigatorAnalysis({ investigator }: { investigator: InvestigatorStatus }) {
  const now = useNow(15_000);
  const overdue =
    investigator.state === "scheduled" &&
    investigator.next_run_at &&
    Date.parse(investigator.next_run_at) + 120_000 < now;
  const state = overdue ? "stalled" : investigator.state;
  const running = state === "running";
  const problem = state === "stalled" || state === "failed" || state === "unavailable";
  return (
    <section
      aria-label="Automatic findings"
      className="flex shrink-0 flex-wrap items-center gap-x-6 gap-y-2 border-b bg-background px-4 py-3 text-xs"
    >
      <div className="space-y-1">
        <p role="status" className="flex items-center gap-2 text-foreground">
          {running ? (
            <LoaderCircle aria-hidden className="size-3.5 motion-safe:animate-spin" />
          ) : (
            <Activity aria-hidden className="size-3.5" />
          )}
          {labels[state]}
        </p>
        <p
          role={problem ? "alert" : undefined}
          className={problem ? "text-destructive" : "text-muted-foreground"}
        >
          {overdue
            ? "Investigator check is overdue. The worker has not confirmed progress."
            : investigator.detail}
        </p>
      </div>
      <div className="space-y-1 text-muted-foreground">
        <p>
          {investigator.next_run_at
            ? `${state === "daily_limit" ? "Resumes" : "Next check"}: ${new Date(investigator.next_run_at).toLocaleString()}`
            : "Next check pending worker startup"}
        </p>
        <p>
          {investigator.last_checked_at
            ? `Last checked: ${new Date(investigator.last_checked_at).toLocaleString()}`
            : "No completed worker check recorded"}
        </p>
      </div>
      <p className="text-muted-foreground">
        {investigator.model} · {investigator.runs_today}/{investigator.run_limit} analyses today ·{" "}
        {investigator.posts_today} Slack send attempts today
      </p>
    </section>
  );
}
