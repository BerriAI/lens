"use client";

import { useQuery } from "@tanstack/react-query";
import type { TimeWindow } from "../../shared/timeRange/timeRange";
import { LoadingState } from "../../shared/LoadingState";
import { Button } from "../../ui/button";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "../../ui/sheet";
import { useProfilesApi } from "../data/LensServices";
import { useOpenSourceTrace } from "../route";
import { formatCount, formatLatency, formatUsd, SOURCE_LABELS } from "./profilesData";

export function ProfileDetailSheet({
  agent,
  window,
  profileId,
  canInvestigate,
  investigateDisabled,
  onInvestigate,
  onClose,
}: {
  agent: string;
  window: TimeWindow;
  profileId: string | null;
  canInvestigate: boolean;
  investigateDisabled: boolean;
  onInvestigate: (profileId: string) => void;
  onClose: () => void;
}) {
  const api = useProfilesApi();
  const openTrace = useOpenSourceTrace();
  const query = useQuery({
    queryKey: ["lens", "profile-detail", api.scope, window, agent, profileId],
    queryFn: ({ signal }) => api.detail(window, agent, profileId ?? "", signal),
    enabled: !!profileId && !!agent,
    staleTime: 30_000,
  });
  const detail = query.data;
  return (
    <Sheet open={!!profileId} onOpenChange={(open) => !open && onClose()}>
      <SheetContent className="w-full overflow-y-auto sm:max-w-lg">
        <SheetHeader>
          <SheetTitle>{detail?.profile.name ?? "Profile"}</SheetTitle>
          <SheetDescription>
            {detail ? `${SOURCE_LABELS[detail.profile.source]} · ${agent}` : agent}
          </SheetDescription>
        </SheetHeader>
        <div className="flex flex-col gap-6 px-4 pb-6">
          {query.isPending && <LoadingState title="Loading profile…" description="Reading runs and model figures." />}
          {query.error && <p role="alert" className="text-sm text-destructive">{query.error.message}</p>}
          {detail && (
            <>
              <dl className="grid grid-cols-3 gap-3 text-sm">
                <div>
                  <dt className="text-xs text-muted-foreground">Runs</dt>
                  <dd className="font-medium tabular-nums">{formatCount(detail.profile.run_count)}</dd>
                </div>
                <div>
                  <dt className="text-xs text-muted-foreground">Requests</dt>
                  <dd className="font-medium tabular-nums">{formatCount(detail.profile.request_count)}</dd>
                </div>
                <div>
                  <dt className="text-xs text-muted-foreground">Known spend</dt>
                  <dd className="font-medium tabular-nums">{formatUsd(detail.profile.spend_usd) ?? "No data"}</dd>
                </div>
              </dl>
              <section className="space-y-2">
                <h3 className="lens-section-label">Models</h3>
                {detail.models.length === 0 ? (
                  <p className="text-sm text-muted-foreground">No model requests by {agent} in these runs.</p>
                ) : (
                  <ul className="divide-y rounded-md border text-sm">
                    {detail.models.map((cell) => (
                      <li key={cell.column_key} className="flex flex-wrap justify-between gap-2 px-3 py-2">
                        <span className="break-all">{cell.column_key.replace(/^(id|group|unmatched):/, "")}</span>
                        <span className="tabular-nums text-muted-foreground">
                          {formatCount(cell.request_count)} requests · {formatUsd(cell.avg_cost_usd) ?? "cost unknown"} avg ·{" "}
                          {formatLatency(cell.p95_latency_ms) ?? "latency unknown"} p95
                        </span>
                      </li>
                    ))}
                  </ul>
                )}
              </section>
              <section className="space-y-2">
                <h3 className="lens-section-label">Recent runs</h3>
                <ul className="space-y-1">
                  {detail.runs.map((run) => (
                    <li key={run.trace_id}>
                      <button
                        type="button"
                        className="w-full rounded-md border px-3 py-2 text-left text-sm hover:bg-muted/50"
                        onClick={() => openTrace({ agent, traceId: run.trace_id, traceRef: run.trace_ref, spanId: "" })}
                      >
                        <span className="line-clamp-2">{run.input_preview || run.trace_id}</span>
                        <span className="text-xs text-muted-foreground">
                          {new Date(run.start_ms).toLocaleString()} · {formatCount(run.request_count)} requests ·{" "}
                          {formatUsd(run.spend_usd) ?? "spend unknown"}
                        </span>
                      </button>
                    </li>
                  ))}
                </ul>
              </section>
              {detail.coverage.runs_resolved < detail.coverage.runs_listed && (
                <p className="text-xs text-muted-foreground">
                  Figures cover {formatCount(detail.coverage.runs_resolved)} of {detail.coverage.truncated && "at least "}
                  {formatCount(detail.coverage.runs_listed)} runs in this range.
                </p>
              )}
              {canInvestigate && (
                <Button disabled={investigateDisabled} onClick={() => onInvestigate(detail.profile.profile_id)}>
                  Investigate profile
                </Button>
              )}
            </>
          )}
        </div>
      </SheetContent>
    </Sheet>
  );
}
