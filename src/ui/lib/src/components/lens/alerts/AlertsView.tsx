"use client";

import { useQuery } from "@tanstack/react-query";
import { Flag, RefreshCw } from "lucide-react";
import { useState } from "react";
import { Button } from "../../ui/button";
import { Skeleton } from "../../ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "../../ui/table";
import { useLensApi } from "../data/LensServices";
import { lensQueries } from "../data/queries";
import type { SignalOverview } from "../model/types";
import { SettingsCard, SettingsSection } from "../settings/SettingsSection";
import { SignalSettings } from "../settings/signals/SignalSettings";
import type { OpenTraceRouting } from "../traces/routing";
import { LensPageHeader } from "../ui/LensPageHeader";

type OpenSignal = OpenTraceRouting["openSignal"];
const PAGE_SIZE = 20;

export function AlertsView({ canConfigure, onOpenSignal }: { canConfigure: boolean; onOpenSignal: OpenSignal }) {
  const api = useLensApi();
  const overview = useQuery(lensQueries.signalOverview(api));
  return (
    <div aria-label="Alerts" role="region" className="flex w-full min-w-0 flex-col">
      <LensPageHeader
        page="alerts"
        title="Alerts"
        description="Review Signal matches and choose what gets sent to Slack"
        actions={
          <Button variant="outline" size="sm" disabled={overview.isFetching} onClick={() => void overview.refetch()}>
            <RefreshCw aria-hidden="true" /> Refresh
          </Button>
        }
      />
      <div className="lens-page-body w-full min-w-0 divide-y divide-border">
        <SettingsSection
          layout="stacked"
          heading="Signal activity"
          description="Signals and Slack work in standalone Lens. An optional LiteLLM Gateway can supply traces and judge models. Lens can only check what the trace captured."
          icon={<Flag aria-hidden="true" className="size-4" />}
        >
          {overview.error ? (
            <p role="alert" className="text-sm text-destructive">
              Could not load Signal activity: {overview.error.message}
            </p>
          ) : overview.data ? (
            <SignalActivity data={overview.data} onOpenSignal={onOpenSignal} />
          ) : (
            <Skeleton aria-label="Loading Signal activity" className="h-40 w-full" />
          )}
        </SettingsSection>
        {canConfigure && <SignalSettings />}
      </div>
    </div>
  );
}

function SignalActivity({ data, onOpenSignal }: { data: SignalOverview; onOpenSignal: OpenSignal }) {
  const [selected, setSelected] = useState<string | null>(null);
  const [shown, setShown] = useState(PAGE_SIZE);
  const groups = new Map<string, { name: string; count: number }>();
  const hits = data.matches.flatMap((trace) => (trace.flags ?? []).map((flag) => ({ trace, flag })));
  for (const { flag } of hits) {
    groups.set(flag.signal_id, { name: flag.name, count: (groups.get(flag.signal_id)?.count ?? 0) + 1 });
  }
  const filter = selected && groups.has(selected) ? selected : null;
  const visible = filter ? hits.filter(({ flag }) => flag.signal_id === filter) : hits;
  const select = (id: string | null) => {
    setSelected(id);
    setShown(PAGE_SIZE);
  };
  return (
    <SettingsCard className="space-y-4">
      <div className="space-y-1">
        <h3 className="text-sm font-medium">Last 24 hours</h3>
        <p className="text-xs text-muted-foreground">
          {data.evaluated} evaluated traces · {data.matches.length} matched · {data.failed} failed evaluations ·{" "}
          {data.partial} partial
        </p>
        {data.truncated && (
          <p role="status" className="text-xs text-muted-foreground">
            Counts cover only the latest {data.limit} results in this window.
          </p>
        )}
        <p className="text-xs text-muted-foreground">
          Matches use your current Signal settings. Failed and incomplete evaluations do not establish that a trace is
          clear.
        </p>
      </div>
      {hits.length > 0 ? (
        <>
          <div role="group" aria-label="Filter by Signal" className="flex flex-wrap gap-2">
            <Button
              variant={filter ? "outline" : "secondary"}
              size="sm"
              aria-pressed={!filter}
              onClick={() => select(null)}
            >
              All Signals ({hits.length})
            </Button>
            {[...groups.entries()].map(([id, group]) => (
              <Button
                key={id}
                variant={filter === id ? "secondary" : "outline"}
                size="sm"
                className="h-auto max-w-full whitespace-normal wrap-anywhere text-left"
                aria-pressed={filter === id}
                onClick={() => select(id)}
              >
                {group.name} ({group.count})
              </Button>
            ))}
          </div>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Signal</TableHead>
                <TableHead>Score</TableHead>
                <TableHead>Trace</TableHead>
                <TableHead>Evaluated</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {visible.slice(0, shown).map(({ trace, flag }) => (
                <TableRow key={JSON.stringify([trace.trace_id, trace.trace_ref, flag.signal_id])}>
                  <TableCell className="font-medium">{flag.name}</TableCell>
                  <TableCell>
                    {new Intl.NumberFormat(undefined, { style: "percent", maximumFractionDigits: 1 }).format(
                      flag.score,
                    )}
                  </TableCell>
                  <TableCell>
                    <Button
                      variant="link"
                      size="sm"
                      className="h-auto p-0 font-mono text-xs"
                      onClick={() => onOpenSignal({ traceId: trace.trace_id, traceRef: trace.trace_ref }, flag)}
                    >
                      {trace.trace_id}
                    </Button>
                  </TableCell>
                  <TableCell>
                    {trace.classified_at && (
                      <time dateTime={trace.classified_at}>{new Date(trace.classified_at).toLocaleString()}</time>
                    )}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
          {shown < visible.length && (
            <Button variant="outline" size="sm" onClick={() => setShown(shown + PAGE_SIZE)}>
              Show more matches
            </Button>
          )}
        </>
      ) : (
        <p className="text-sm text-muted-foreground">
          {data.evaluated ? "No Signal matches in this window." : "No evaluated traces in this window."}
        </p>
      )}
    </SettingsCard>
  );
}
