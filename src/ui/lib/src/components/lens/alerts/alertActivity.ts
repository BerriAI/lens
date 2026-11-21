import type { AlertRuleMatch, AlertRuleOverview, SignalOverview } from "../model/types";
import { durationText } from "../model/format";

export const HOUR_MS = 3_600_000;
const COLORS = [
  "var(--lens-brand)",
  "var(--lens-amber)",
  "var(--lens-violet)",
  "var(--lens-cyan)",
  "var(--lens-rose)",
];

type Trace = SignalOverview["matches"][number];
export type AlertHit = { trace: Trace; flag: NonNullable<Trace["flags"]>[number]; rule?: AlertRuleMatch };

export function ruleName(rule: AlertRuleMatch) {
  return rule.kind === "run_duration" ? "Run duration" : "Model error";
}

export function ruleDetail(rule: AlertRuleMatch) {
  return rule.kind === "run_duration"
    ? `${durationText(Math.ceil(rule.duration_ms / 1000))} · Limit ${rule.threshold_minutes} min`
    : "A model call failed";
}

export function alertActivity(data: SignalOverview, rules?: AlertRuleOverview) {
  const ruleHits: AlertHit[] = (rules?.matches ?? []).map((rule) => ({
    rule,
    trace: {
      trace_id: rule.trace_id,
      trace_ref: rule.trace_ref,
      classified_at: rule.detected_at,
      status: "classified",
      model: "",
      flags: [],
    },
    flag: {
      signal_id: `rule:${rule.kind}`,
      name: ruleName(rule),
      score: 0,
      evidence: { span_id: rule.span_id, quote: "" },
    },
  }));
  const hits: AlertHit[] = [
    ...data.matches.flatMap((trace) => (trace.flags ?? []).map((flag) => ({ trace, flag }))),
    ...ruleHits,
  ].toSorted((a, b) => Date.parse(b.trace.classified_at ?? "") - Date.parse(a.trace.classified_at ?? ""));
  const names = [...new Map(hits.map(({ flag }) => [flag.signal_id, flag.name])).entries()];
  const signals = names
    .map(([id, name], index) => ({
      id,
      name,
      color: COLORS[index % COLORS.length],
      count: hits.filter(({ flag }) => flag.signal_id === id).length,
    }))
    .toSorted((a, b) => b.count - a.count);
  const since = Math.max(Date.parse(data.since), Date.parse(rules?.since ?? data.since));
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
