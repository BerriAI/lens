"use client";

import { ArrowRight, ChevronRight, RefreshCw, Search, X } from "lucide-react";
import { useMemo, useState } from "react";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "../../ui/sheet";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../ui/select";
import { useNow } from "../../../hooks/useNow";
import { agoLabel } from "../model/format";
import type { AlertRuleOverview, SignalOverview } from "../model/types";
import { useAlertsRoute } from "../route";
import { useOnboarding } from "../onboarding/OnboardingContext";
import type { OpenTraceRouting } from "../traces/routing";
import { alertActivity, ruleDetail, type AlertActivity } from "./alertActivity";
import { AlertEvidence } from "./AlertEvidence";
import styles from "./Alerts.module.css";

const PAGE_SIZE = 20;
const score = new Intl.NumberFormat(undefined, { style: "percent", maximumFractionDigits: 1 });
const time = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
const period = (bucket: AlertActivity["buckets"][number]) =>
  `${time.format(bucket.start)} to ${time.format(bucket.end)}`;

function ActivityChart({
  activity,
  selected,
  onSelect,
}: {
  activity: AlertActivity;
  selected: number | null;
  onSelect: (index: number | null) => void;
}) {
  const peak = Math.max(1, ...activity.buckets.map((bucket) => bucket.count));
  return (
    <section aria-label="Alerts over time" className={styles.panel}>
      <div className={styles.sectionHeading}>
        <h2>Alerts over time</h2>
        <span>Hourly</span>
      </div>
      <div className={styles.chart}>
        <div aria-hidden="true" className={styles.yAxis}>
          <span>{peak}</span>
          <span>0</span>
        </div>
        <div className={styles.bars}>
          {activity.buckets.map((bucket) => (
            <button
              key={bucket.index}
              type="button"
              className={styles.bar}
              aria-label={`${period(bucket)}: ${bucket.count} ${bucket.count === 1 ? "alert" : "alerts"}`}
              aria-pressed={selected === bucket.index}
              title={`${period(bucket)}: ${bucket.count} alerts`}
              onClick={() => onSelect(selected === bucket.index ? null : bucket.index)}
              style={{
                opacity: selected !== null && selected !== bucket.index ? 0.3 : 1,
              }}
            >
              <span
                aria-hidden="true"
                className={styles.stack}
                style={{ height: `${(bucket.count / peak) * 100}%` }}
              >
                {bucket.signals
                  .filter((signal) => signal.count > 0)
                  .map((signal) => (
                    <span key={signal.id} style={{ flex: signal.count, background: signal.color }} />
                  ))}
              </span>
            </button>
          ))}
        </div>
      </div>
      <div aria-hidden="true" className={styles.axis}>
        <span>24h ago</span>
        <span>18h</span>
        <span>12h</span>
        <span>6h</span>
        <span>Now</span>
      </div>
      <div className={styles.legend}>
        {activity.signals.map((signal) => (
          <span key={signal.id}>
            <i style={{ background: signal.color }} />
            {signal.name}
          </span>
        ))}
      </div>
      {activity.unplotted > 0 && (
        <p className="mt-3 text-xs text-muted-foreground">
          {activity.unplotted} alerts have no timestamp in this window
        </p>
      )}
    </section>
  );
}

export function AlertDashboard({
  accessToken,
  data,
  rules,
  onOpenTrace,
  onOpenSignal,
  refreshing,
  onRefresh,
}: {
  accessToken: string;
  data: SignalOverview;
  rules?: AlertRuleOverview;
  onOpenTrace: OpenTraceRouting["openTrace"];
  onOpenSignal: OpenTraceRouting["openSignal"];
  refreshing: boolean;
  onRefresh: () => void;
}) {
  const route = useAlertsRoute();
  const { canInvestigate } = useOnboarding();
  const [selected, setSelected] = useState<AlertActivity["hits"][number] | null>(null);
  const now = useNow(30_000);
  const activity = useMemo(() => alertActivity(data, rules), [data, rules]);
  const { hits, signals } = activity;
  const hour = activity.buckets.find((bucket) => bucket.index === route.hour);
  const filterKey = JSON.stringify([route.signal, route.search, route.hour]);
  const [pagination, setPagination] = useState({ filter: filterKey, shown: PAGE_SIZE });
  const shown = pagination.filter === filterKey ? pagination.shown : PAGE_SIZE;
  const mostCommon = signals[0];
  const tied = signals.filter((signal) => signal.count === mostCommon?.count).length - 1;
  const items = [
    { value: "", label: "All alerts" },
    ...signals.map((signal) => ({ value: signal.id, label: signal.name })),
    ...(route.signal && !signals.some((signal) => signal.id === route.signal)
      ? [{ value: route.signal, label: "Selected signal" }]
      : []),
  ];
  const search = route.search.trim().toLocaleLowerCase();
  const visible = hits.filter(
    ({ trace, flag }) =>
      (!route.signal || flag.signal_id === route.signal) &&
      (!search || `${flag.name} ${trace.trace_id}`.toLocaleLowerCase().includes(search)) &&
      (!hour ||
        (Date.parse(trace.classified_at ?? "") >= hour.start &&
          Date.parse(trace.classified_at ?? "") < hour.end)),
  );
  const filtered = Boolean(route.signal || route.search || hour);
  return (
    <>
      <div className={`lens-page-body ${styles.dashboard}`}>
        <section aria-label="Last 24 hours">
          <div className={styles.window}>
            <p>Last 24 hours</p>
            <Button
              variant="ghost"
              size="icon-xs"
              aria-label="Refresh"
              title="Refresh alerts"
              disabled={refreshing}
              onClick={onRefresh}
            >
              <RefreshCw aria-hidden="true" />
            </Button>
          </div>
          <dl className={styles.metrics}>
            <div>
              <dt>Alerts raised</dt>
              <dd>{hits.length.toLocaleString()}</dd>
              <small>
                Across {activity.traces.toLocaleString()} {activity.traces === 1 ? "trace" : "traces"}
              </small>
            </div>
            <div>
              <dt>Model-evaluated traces</dt>
              <dd>{data.evaluated.toLocaleString()}</dd>
            </div>
            <div>
              <dt>Most common alert</dt>
              <dd className={styles.common}>{mostCommon?.name ?? "None yet"}</dd>
              <small>
                {mostCommon ? (
                  <>
                    <span>
                      {mostCommon.count} of {hits.length} alerts
                    </span>
                    {tied > 0
                      ? ` · Tied with ${tied} ${tied === 1 ? "other" : "others"}`
                      : ` · ${score.format(mostCommon.count / hits.length)}`}
                  </>
                ) : (
                  "Waiting for alerts"
                )}
              </small>
            </div>
          </dl>
          {(data.truncated || rules?.truncated || data.failed > 0 || data.partial > 0) && (
            <div className="mt-3 space-y-1 text-xs text-muted-foreground">
              {data.truncated && (
                <p role="status">
                  Model signal counts cover only the latest {data.limit} results in this window.
                </p>
              )}
              {rules?.truncated && (
                <p role="status">Rule counts cover only the latest {rules.limit} alerts in this window.</p>
              )}
              {(data.failed > 0 || data.partial > 0) && (
                <p>Some traces could not be fully evaluated. Alerts may be incomplete.</p>
              )}
            </div>
          )}
        </section>
        {hits.length > 0 && (
          <div className={styles.chartGrid}>
            <ActivityChart activity={activity} selected={hour?.index ?? null} onSelect={route.setHour} />
            <section aria-label="Alerts by type" className={styles.panel}>
              <div className={styles.sectionHeading}>
                <h2>By alert</h2>
                <span>{hits.length} alerts</span>
              </div>
              <div className={styles.ranking}>
                {signals.map((signal) => (
                  <button
                    key={signal.id}
                    type="button"
                    aria-label={`Filter ${signal.name}, ${signal.count} alerts`}
                    aria-pressed={route.signal === signal.id}
                    onClick={() => route.setSignal(route.signal === signal.id ? "" : signal.id)}
                  >
                    <span>
                      <span>
                        <i style={{ background: signal.color }} />
                        {signal.name}
                      </span>
                      <strong>{signal.count}</strong>
                    </span>
                    <span className={styles.rankTrack}>
                      <span
                        style={{
                          width: `${(signal.count / mostCommon.count) * 100}%`,
                          background: signal.color,
                        }}
                      />
                    </span>
                  </button>
                ))}
              </div>
            </section>
          </div>
        )}
        <section aria-label="Alert matches" className={styles.panel}>
          <div className={styles.sectionHeading}>
            <h2>Recent alerts</h2>
            <span>Most recent first</span>
          </div>
          <div className={styles.filters}>
            <div className={styles.search}>
              <Search aria-hidden="true" size={14} />
              <Input
                aria-label="Search matches"
                placeholder="Search alerts"
                value={route.search}
                onChange={(event) => route.setSearch(event.target.value)}
              />
            </div>
            <Select
              items={items}
              value={route.signal}
              onValueChange={(value) => value !== null && route.setSignal(value)}
            >
              <SelectTrigger aria-label="Filter by alert" size="sm" className="w-44">
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
            {canInvestigate && data.matches.some((trace) => trace.flags?.some((flag) => flag.signal_id === route.signal)) && (
              <Button
                variant="outline"
                size="sm"
                title="Start an investigation of runs from the last 24 hours with this saved signal"
                onClick={() => route.investigateSignal(route.signal)}
              >
                Investigate signal
                <ArrowRight aria-hidden="true" />
              </Button>
            )}
            {hour && (
              <Button variant="outline" size="sm" onClick={() => route.setHour(null)}>
                {period(hour)}
                <X aria-hidden="true" />
              </Button>
            )}
            {filtered && (
              <Button variant="ghost" size="sm" onClick={route.clearFilters}>
                Clear filters
              </Button>
            )}
          </div>
          {visible.length > 0 ? (
            <div className={styles.alerts}>
              {visible.slice(0, shown).map(({ trace, flag, rule }) => (
                <button
                  type="button"
                  key={JSON.stringify([trace.trace_id, trace.trace_ref, flag.signal_id])}
                  aria-label={`Open ${flag.name} in trace ${trace.trace_id}`}
                  className={styles.alert}
                  onClick={() => setSelected({ trace, flag, rule })}
                >
                  <i style={{ background: signals.find((signal) => signal.id === flag.signal_id)?.color }} />
                  <span className={styles.alertText}>
                    <strong>{flag.name}</strong>
                    <span>
                      <code title={trace.trace_id}>{trace.trace_id}</code>
                    </span>
                  </span>
                  <span className={styles.alertMeta}>
                    {trace.classified_at && (
                      <time
                        dateTime={trace.classified_at}
                        title={new Date(trace.classified_at).toLocaleString()}
                      >
                        {agoLabel(Date.parse(trace.classified_at), now)}
                      </time>
                    )}
                    <span>{rule ? ruleDetail(rule) : score.format(flag.score)}</span>
                  </span>
                  <ChevronRight aria-hidden="true" size={15} />
                </button>
              ))}
            </div>
          ) : (
            <div className={styles.empty}>
              <p className="text-sm font-medium">
                {filtered ? "No matches for these filters" : "No alerts in this window"}
              </p>
              <p className="mt-2 text-sm text-muted-foreground">
                {filtered
                  ? "Try another alert or trace ID."
                  : "Alerts appear here when a run matches an enabled rule or model signal."}
              </p>
            </div>
          )}
          <footer className={styles.footer}>
            <span>
              {Math.min(shown, visible.length)} of {visible.length}{" "}
              {visible.length === 1 ? "alert" : "alerts"}
            </span>
            {shown < visible.length && (
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setPagination({ filter: filterKey, shown: shown + PAGE_SIZE })}
              >
                Show more matches
                <ArrowRight aria-hidden="true" />
              </Button>
            )}
          </footer>
        </section>
      </div>
      <Sheet open={selected !== null} onOpenChange={(open) => !open && setSelected(null)}>
        <SheetContent className="lens-shell-overlay w-full! gap-0 sm:max-w-xl!">
          <SheetHeader className="border-b px-6 py-5 pr-14">
            <SheetTitle>Alert evidence</SheetTitle>
            <SheetDescription>{selected?.flag.name}</SheetDescription>
          </SheetHeader>
          {selected && (
            <AlertEvidence
              accessToken={accessToken}
              selected={selected}
              onOpenTrace={onOpenTrace}
              onOpenSignal={onOpenSignal}
            />
          )}
        </SheetContent>
      </Sheet>
    </>
  );
}
