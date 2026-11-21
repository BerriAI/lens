"use client";

import { useQuery } from "@tanstack/react-query";
import { Bell, RefreshCw, Search, Settings2 } from "lucide-react";
import { useState, type ReactNode } from "react";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../ui/select";
import { Skeleton } from "../../ui/skeleton";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle, SheetTrigger } from "../../ui/sheet";
import { useNow } from "../../../hooks/useNow";
import { agoLabel } from "../model/format";
import { useLensApi } from "../data/LensServices";
import { lensQueries } from "../data/queries";
import type { SignalOverview } from "../model/types";
import { useAlertsRoute } from "../route";
import { SignalSettings } from "../settings/signals/SignalSettings";
import type { OpenTraceRouting } from "../traces/routing";
import { SignalPills } from "../traces/ui/SignalPills";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "../../ui/table";
import styles from "../traces/TraceAppearance.module.css";

type OpenSignal = OpenTraceRouting["openSignal"];
const PAGE_SIZE = 20;

export function AlertsView({ canConfigure, onOpenSignal }: { canConfigure: boolean; onOpenSignal: OpenSignal }) {
  const api = useLensApi();
  const route = useAlertsRoute();
  const overview = useQuery(lensQueries.signalOverview(api));
  const actions = (
    <>
      <Button
        variant="ghost"
        size="icon-sm"
        className="h-10 w-10 shrink-0 rounded-none border-l"
        aria-label="Refresh"
        title="Refresh alerts"
        disabled={overview.isFetching}
        onClick={() => void overview.refetch()}
      >
        <RefreshCw aria-hidden="true" />
      </Button>
      {canConfigure && (
        <>
          <SheetTrigger
            render={<Button variant="ghost" size="sm" className="h-10 rounded-none border-l px-3 text-xs" />}
            onClick={() => route.setSettings("signals")}
          >
            <Settings2 aria-hidden="true" /> Configure signals
          </SheetTrigger>
          <SheetTrigger
            render={<Button variant="ghost" size="sm" className="h-10 rounded-none border-l px-3 text-xs" />}
            onClick={() => route.setSettings("slack")}
          >
            <Bell aria-hidden="true" /> Slack notifications
          </SheetTrigger>
        </>
      )}
    </>
  );
  return (
    <Sheet open={canConfigure && route.settings !== null} onOpenChange={(open) => !open && route.setSettings(null)}>
      <div aria-label="Alerts" role="region" className="flex min-h-0 w-full min-w-0 flex-1 flex-col bg-card">
        {(!overview.data || overview.error) && (
          <div className="lens-toolbar flex flex-wrap justify-end border-b">{actions}</div>
        )}
        <div className="flex min-h-0 flex-1 flex-col">
          {overview.error ? (
            <div role="alert" className="space-y-3 py-8 text-sm">
              <p className="font-medium">Could not load Signal activity</p>
              <p className="text-muted-foreground">{overview.error.message}</p>
              <Button variant="outline" size="sm" onClick={() => void overview.refetch()}>
                Try again
              </Button>
            </div>
          ) : overview.data ? (
            <SignalActivity data={overview.data} onOpenSignal={onOpenSignal} actions={actions} />
          ) : (
            <Skeleton aria-label="Loading Signal activity" className="h-40 w-full" />
          )}
        </div>
      </div>
      {canConfigure && (
        <SheetContent className="lens-shell-overlay w-full! gap-0 sm:max-w-2xl!">
          <SheetHeader className="shrink-0 border-b px-6 py-5 pr-14">
            <SheetTitle>{route.settings === "slack" ? "Slack notifications" : "Configure signals"}</SheetTitle>
            <SheetDescription>
              {route.settings === "slack"
                ? "Send selected signal matches to a Slack channel."
                : "Choose what Lens checks in new traces across all agents and teams."}
            </SheetDescription>
          </SheetHeader>
          <div className="min-h-0 flex-1 overflow-y-auto px-6 py-6">
            {route.settings && (
              <SignalSettings section={route.settings} onConfigureSignals={() => route.setSettings("signals")} />
            )}
          </div>
        </SheetContent>
      )}
    </Sheet>
  );
}

function SignalActivity({
  data,
  onOpenSignal,
  actions,
}: {
  data: SignalOverview;
  onOpenSignal: OpenSignal;
  actions: ReactNode;
}) {
  const route = useAlertsRoute();
  const now = useNow(30_000);
  const filterKey = JSON.stringify([route.signal, route.search]);
  const [pagination, setPagination] = useState({
    filter: filterKey,
    shown: PAGE_SIZE,
  });
  const shown = pagination.filter === filterKey ? pagination.shown : PAGE_SIZE;
  const hits = data.matches.flatMap((trace) => (trace.flags ?? []).map((flag) => ({ trace, flag })));
  const signals = [...new Map(hits.map(({ flag }) => [flag.signal_id, flag.name])).entries()];
  const signalCounts = signals.map(([id, name]) => ({
    id,
    name,
    count: hits.filter(({ flag }) => flag.signal_id === id).length,
  }));
  const mostCommon = signalCounts.toSorted((a, b) => b.count - a.count)[0];
  const tiedSignals = signalCounts.filter((signal) => signal.count === mostCommon?.count).length - 1;
  const items = [
    { value: "", label: "All signals" },
    ...signals.map(([value, label]) => ({ value, label })),
    ...(route.signal && !signals.some(([id]) => id === route.signal)
      ? [{ value: route.signal, label: "Selected signal" }]
      : []),
  ];
  const search = route.search.trim().toLocaleLowerCase();
  const visible = hits.filter(
    ({ trace, flag }) =>
      (!route.signal || flag.signal_id === route.signal) &&
      (!search || `${flag.name} ${trace.trace_id}`.toLocaleLowerCase().includes(search)),
  );
  const filtered = Boolean(route.signal || route.search);
  return (
    <>
      <div className="lens-toolbar flex min-w-0 shrink-0 flex-wrap items-stretch border-b">
        <div className="relative h-10 min-w-[min(100%,16rem)] flex-1 basis-full sm:basis-64">
          <Search
            aria-hidden="true"
            className="pointer-events-none absolute top-3 left-3 size-3.5 text-muted-foreground"
          />
          <Input
            aria-label="Search matches"
            placeholder="Search signals or trace IDs"
            value={route.search}
            onChange={(event) => route.setSearch(event.target.value)}
            className="h-full rounded-none border-0 bg-transparent pl-9 font-mono text-xs shadow-none focus-visible:ring-inset dark:bg-transparent"
          />
        </div>
        <div className="flex h-10 min-w-0 items-center border-l px-2">
          <Select
            items={items}
            value={route.signal}
            onValueChange={(value) => value !== null && route.setSignal(value)}
          >
            <SelectTrigger
              aria-label="Filter by Signal"
              size="sm"
              className="h-7 w-40 bg-background font-mono text-[11px]"
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {items.map((item) => (
                <SelectItem key={item.value} value={item.value}>
                  {item.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        {actions}
      </div>
      <section
        aria-label="Last 24 hours"
        className="flex shrink-0 flex-wrap items-center justify-between gap-x-6 gap-y-1 border-b px-3 py-1.5"
      >
        <dl className={`${styles.counts} min-w-0 max-w-full`}>
          <div>
            <dt>Alerts raised</dt>
            <dd>{hits.length.toLocaleString()}</dd>
          </div>
          <div>
            <dt>Evaluated traces</dt>
            <dd>{data.evaluated.toLocaleString()}</dd>
          </div>
          <div className="min-w-0">
            <dt>Most common signal</dt>
            <dd
              className="min-w-0"
              title={
                mostCommon
                  ? `${mostCommon.count} of ${hits.length} alerts${tiedSignals > 0 ? `, tied with ${tiedSignals} others` : ""}`
                  : undefined
              }
            >
              {mostCommon ? (
                <span className="inline-flex max-w-full items-center gap-1.5">
                  <span className="max-w-48 truncate">{mostCommon.name}</span>
                  <span className="text-muted-foreground">{mostCommon.count}</span>
                  <span className="sr-only">
                    {mostCommon.count} of {hits.length} alerts
                  </span>
                </span>
              ) : (
                "None yet"
              )}
            </dd>
          </div>
        </dl>
        <p className="text-xs text-muted-foreground">Last 24 hours · All agents and teams</p>
      </section>
      <section aria-label="Signal matches" className="min-h-0 flex-1 overflow-auto">
        {visible.length > 0 ? (
          <Table className="table-fixed border-collapse">
            <TableHeader className="sticky top-0 z-sticky">
              <TableRow className="h-8 hover:bg-transparent [&>th]:h-8 [&>th]:px-3 [&>th]:text-muted-foreground">
                <TableHead className="w-[45%] sm:w-[35%]">Signal</TableHead>
                <TableHead className="hidden sm:table-cell">Trace ID</TableHead>
                <TableHead className="w-20 text-right">Score</TableHead>
                <TableHead className="w-24 text-right">Raised</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {visible.slice(0, shown).map(({ trace, flag }) => (
                <TableRow
                  key={JSON.stringify([trace.trace_id, trace.trace_ref, flag.signal_id])}
                  onClick={() => onOpenSignal({ traceId: trace.trace_id, traceRef: trace.trace_ref }, flag)}
                  className="h-10 cursor-pointer border-border/60 hover:bg-trace-row-hover [&>td]:px-3 [&>td]:py-1.5"
                >
                  <TableCell>
                    <button
                      type="button"
                      aria-label={`Open ${flag.name} in trace ${trace.trace_id}`}
                      className="flex w-full min-w-0 flex-col items-start gap-1 rounded-sm text-left outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    >
                      <SignalPills flags={[flag]} />
                      <span className="max-w-full truncate font-mono text-[11px] text-muted-foreground sm:hidden">
                        {trace.trace_id}
                      </span>
                    </button>
                  </TableCell>
                  <TableCell
                    className="hidden truncate font-mono text-xs text-muted-foreground sm:table-cell"
                    title={trace.trace_id}
                  >
                    {trace.trace_id}
                  </TableCell>
                  <TableCell className="text-right font-mono text-xs tabular-nums">
                    {new Intl.NumberFormat(undefined, {
                      style: "percent",
                      maximumFractionDigits: 1,
                    }).format(flag.score)}
                  </TableCell>
                  <TableCell className="text-right text-xs text-muted-foreground">
                    {trace.classified_at && (
                      <time dateTime={trace.classified_at} title={new Date(trace.classified_at).toLocaleString()}>
                        {agoLabel(Date.parse(trace.classified_at), now)}
                      </time>
                    )}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        ) : (
          <div className="flex flex-col items-center gap-2 px-4 py-16 text-center">
            <p className="text-sm font-medium">
              {filtered
                ? "No matches for these filters"
                : data.evaluated
                  ? "No Signal matches in this window."
                  : "No evaluated traces in this window."}
            </p>
            <p className="max-w-md text-sm leading-6 text-muted-foreground">
              {filtered
                ? "Try another signal or trace ID."
                : data.evaluated
                  ? "Matches will appear here when a trace reaches your signal threshold."
                  : "Matches will appear here as new traces are evaluated."}
            </p>
            {filtered && (
              <Button variant="outline" size="sm" className="mt-2" onClick={route.clearFilters}>
                Clear filters
              </Button>
            )}
          </div>
        )}
      </section>
      {(data.truncated || data.failed > 0 || data.partial > 0) && (
        <div className="space-y-1 border-t px-3 py-2 text-xs text-muted-foreground">
          {data.truncated && <p role="status">Counts cover only the latest {data.limit} results in this window.</p>}
          {(data.failed > 0 || data.partial > 0) && (
            <p>Some traces could not be fully evaluated. Alerts may be incomplete.</p>
          )}
        </div>
      )}
      <footer className="flex min-h-8 shrink-0 items-center justify-between gap-3 border-t bg-muted/30 px-3 py-1 font-mono text-[11px] text-muted-foreground">
        <span>
          {Math.min(shown, visible.length)} of {visible.length} {visible.length === 1 ? "alert" : "alerts"}
        </span>
        {shown < visible.length && (
          <Button
            variant="ghost"
            size="xs"
            className="font-sans"
            onClick={() => setPagination({ filter: filterKey, shown: shown + PAGE_SIZE })}
          >
            Show more matches
          </Button>
        )}
      </footer>
    </>
  );
}
