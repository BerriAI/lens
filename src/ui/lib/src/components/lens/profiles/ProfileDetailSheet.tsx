"use client";

import { useQuery } from "@tanstack/react-query";
import { ChevronRight } from "lucide-react";
import type { TimeWindow } from "../../shared/timeRange/timeRange";
import { cn } from "../../../lib/cva.config";
import { LoadingState } from "../../shared/LoadingState";
import { Button } from "../../ui/button";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "../../ui/sheet";
import { formatActivityTimestamp, formatRunTimestamp } from "../../../utils/activityTimestamp";
import { useProfilesApi } from "../data/LensServices";
import { useOpenSourceTrace } from "../route";
import { formatCount, formatLatency, formatUsd, modelLabel, SOURCE_LABELS } from "./profilesData";

const NUM = "shrink-0 text-right tabular-nums";
const RUN_GRID = "grid h-9 grid-cols-[6.5rem_minmax(0,1fr)_3.5rem_4.5rem_0.75rem] items-center gap-3 px-2 text-xs";

const unavailable = <span className="text-muted-foreground">Unavailable</span>;

const unknown = (value: string | null) => value ?? <span className="text-muted-foreground">Unknown</span>;

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
  const matrix = useQuery({
    queryKey: ["lens", "profile-matrix", api.scope, window, agent],
    queryFn: ({ signal }) => api.matrix(window, agent, signal),
    enabled: !!profileId && !!agent,
    staleTime: 30_000,
  });
  const detail = query.error ? undefined : query.data;
  const deployments = matrix.data?.models ?? [];
  return (
    <Sheet open={!!profileId} onOpenChange={(open) => !open && onClose()}>
      <SheetContent className="w-full overflow-y-auto sm:max-w-xl">
        <SheetHeader>
          <SheetTitle>{detail?.profile.name ?? "Profile"}</SheetTitle>
          <SheetDescription>{detail ? `${SOURCE_LABELS[detail.profile.source]} · ${agent}` : agent}</SheetDescription>
        </SheetHeader>
        <div className="flex flex-col gap-6 px-4 pb-6">
          {query.isPending && <LoadingState title="Loading profile…" description="Reading runs and model figures." />}
          {query.error && (
            <p role="alert" className="text-sm text-destructive">
              {query.error.message}
            </p>
          )}
          {detail && (
            <>
              {detail.definition?.description && <p className="text-sm">{detail.definition.description}</p>}
              <dl className="flex flex-wrap gap-x-6 gap-y-2 text-xs">
                {[
                  { label: "Runs", value: formatCount(detail.profile.run_count) },
                  { label: "Requests", value: formatCount(detail.profile.request_count) },
                  { label: "Known spend", value: formatUsd(detail.profile.spend_usd) ?? "Unknown" },
                ].map((item) => (
                  <div key={item.label}>
                    <dt className="text-muted-foreground">{item.label}</dt>
                    <dd className="text-sm font-medium tabular-nums">{item.value}</dd>
                  </div>
                ))}
              </dl>
              <section aria-label="Models" className="flex flex-col">
                <h3 className="lens-section-label mb-1">Models</h3>
                {detail.models.length === 0 ? (
                  <p className="text-xs text-muted-foreground">
                    {detail.coverage.runs_resolved < detail.coverage.runs_listed
                      ? "Model figures are unavailable for these runs."
                      : `No model requests by ${agent} in these runs.`}
                  </p>
                ) : (
                  <table className="w-full table-fixed text-xs">
                    <thead>
                      <tr className="h-8 border-b border-border/60 text-muted-foreground">
                        <th scope="col" className="px-2 text-left font-normal">
                          Model
                        </th>
                        <th scope="col" className="w-20 px-2 text-right font-normal">
                          Requests
                        </th>
                        <th scope="col" className="w-20 px-2 text-right font-normal">
                          Avg cost
                        </th>
                        <th scope="col" className="w-20 px-2 text-right font-normal">
                          p95
                        </th>
                      </tr>
                    </thead>
                    <tbody>
                      {detail.models.map((cell) => {
                        const label = modelLabel(cell.column_key, deployments);
                        return (
                          <tr key={cell.column_key} className="h-9 border-b border-border/60 last:border-0">
                            <th scope="row" className="px-2 text-left font-normal">
                              <span className="block truncate font-mono" title={label.name}>
                                {label.name}
                              </span>
                              {label.detail && label.detail !== label.name && (
                                <span className="block truncate text-muted-foreground" title={label.detail}>
                                  {label.detail}
                                </span>
                              )}
                            </th>
                            <td className={`px-2 ${NUM}`}>{formatCount(cell.request_count)}</td>
                            <td className={`px-2 ${NUM}`}>{unknown(formatUsd(cell.avg_cost_usd))}</td>
                            <td className={`px-2 ${NUM}`}>{unknown(formatLatency(cell.p95_latency_ms))}</td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                )}
              </section>
              <section aria-label="Recent runs" className="flex flex-col">
                <h3 className="lens-section-label mb-1">Recent runs</h3>
                {detail.runs.length === 0 ? (
                  <p className="text-xs text-muted-foreground">No runs to show.</p>
                ) : (
                  <>
                    <div aria-hidden="true" className={cn(RUN_GRID, "h-8 border-b border-border/60 text-muted-foreground")}>
                      <span>Time</span>
                      <span>Input</span>
                      <span className="text-right">Requests</span>
                      <span className="text-right">Cost</span>
                      <span />
                    </div>
                    <ul>
                      {detail.runs.map((run) => {
                        const started = new Date(run.start_ms).toISOString();
                        return (
                          <li key={run.trace_ref || run.trace_id} className="border-b border-border/60 last:border-0">
                            <button
                              type="button"
                              className={cn(RUN_GRID, "w-full text-left hover:bg-trace-row-hover focus-visible:outline-2 focus-visible:outline-ring")}
                              onClick={() => openTrace({ agent, traceId: run.trace_id, traceRef: run.trace_ref, spanId: "" })}
                            >
                              <span className="font-mono text-muted-foreground tabular-nums" title={formatActivityTimestamp(started)}>
                                {formatRunTimestamp(started)}
                              </span>
                              {run.input_preview ? (
                                <span className="truncate">{run.input_preview}</span>
                              ) : (
                                <span className="truncate font-mono text-muted-foreground">{run.trace_id}</span>
                              )}
                              <span className={NUM}>{run.resolved ? formatCount(run.request_count) : unavailable}</span>
                              <span className={NUM}>{run.resolved ? unknown(formatUsd(run.spend_usd)) : unavailable}</span>
                              <ChevronRight aria-hidden="true" className="size-3 text-muted-foreground/60" />
                            </button>
                          </li>
                        );
                      })}
                    </ul>
                  </>
                )}
              </section>
              {detail.coverage.runs_resolved < detail.coverage.runs_listed && (
                <p className="text-xs text-muted-foreground">
                  Figures cover {formatCount(detail.coverage.runs_resolved)} of {detail.coverage.truncated && "at least "}
                  {formatCount(detail.coverage.runs_listed)} runs in this range.
                </p>
              )}
              {canInvestigate && (
                <div>
                  <Button size="sm" disabled={investigateDisabled} onClick={() => onInvestigate(detail.profile.profile_id)}>
                    Investigate these runs
                  </Button>
                </div>
              )}
            </>
          )}
        </div>
      </SheetContent>
    </Sheet>
  );
}
