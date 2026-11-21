import type { SignalOverview } from "../model/types";

export const HOUR_MS = 3_600_000;
const COLORS = [
  "var(--lens-brand)",
  "var(--lens-amber)",
  "var(--lens-violet)",
  "var(--lens-cyan)",
  "var(--lens-rose)",
];

export function alertActivity(data: SignalOverview) {
  const hits = data.matches.flatMap((trace) => (trace.flags ?? []).map((flag) => ({ trace, flag })));
  const names = [...new Map(hits.map(({ flag }) => [flag.signal_id, flag.name])).entries()];
  const signals = names
    .map(([id, name], index) => ({
      id,
      name,
      color: COLORS[index % COLORS.length],
      count: hits.filter(({ flag }) => flag.signal_id === id).length,
    }))
    .toSorted((a, b) => b.count - a.count);
  const since = Date.parse(data.since);
  const buckets = Array.from({ length: 24 }, (_, index) => {
    const start = since + index * HOUR_MS;
    const end = start + HOUR_MS;
    const matches = hits.filter(({ trace }) => {
      const at = Date.parse(trace.classified_at ?? "");
      return at >= start && at < end;
    });
    return {
      index,
      start,
      end,
      count: matches.length,
      signals: signals.map((signal) => ({
        ...signal,
        count: matches.filter(({ flag }) => flag.signal_id === signal.id).length,
      })),
    };
  });
  return {
    hits,
    signals,
    buckets,
    traces: new Set(hits.map(({ trace }) => JSON.stringify([trace.trace_id, trace.trace_ref]))).size,
    unplotted: hits.length - buckets.reduce((total, bucket) => total + bucket.count, 0),
  };
}

export type AlertActivity = ReturnType<typeof alertActivity>;
