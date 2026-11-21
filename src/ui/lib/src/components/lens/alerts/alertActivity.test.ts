import { describe, expect, it } from "vitest";
import type { SignalOverview } from "../model/types";
import { alertActivity, HOUR_MS } from "./alertActivity";

const since = new Date("2026-01-01T00:00:00Z").getTime();
const timeout = { signal_id: "timeout", name: "Timeout", score: 0.9 };
const refusal = { signal_id: "refusal", name: "Refusal", score: 0.8 };
const overview = (matches: SignalOverview["matches"]): SignalOverview => ({
  since: new Date(since).toISOString(),
  evaluated: 10,
  failed: 0,
  partial: 0,
  limit: 500,
  truncated: false,
  matches,
});

describe("alert activity", () => {
  it("plots new rule alerts when the signal overview was fetched earlier", () => {
    const activity = alertActivity(overview([]), {
      since: new Date(since + 60_000).toISOString(),
      limit: 500,
      truncated: false,
      matches: [
        {
          kind: "model_error",
          trace_id: "new-run",
          span_id: "failed-call",
          detected_at: new Date(since + 24 * HOUR_MS + 30_000).toISOString(),
        },
      ],
    });
    expect(activity.buckets.at(-1)?.count).toBe(1);
    expect(activity.unplotted).toBe(0);
  });
  it("combines rules and model signals chronologically without merging tenant identities", () => {
    const activity = alertActivity(
      overview([
        {
          trace_id: "same",
          trace_ref: "a",
          status: "classified",
          classified_at: new Date(since + 1000).toISOString(),
          flags: [timeout],
        },
      ]),
      {
        since: new Date(since).toISOString(),
        limit: 500,
        truncated: false,
        matches: [
          {
            kind: "run_duration",
            trace_id: "same",
            trace_ref: "b",
            span_id: "latest",
            detected_at: new Date(since + 2000).toISOString(),
            duration_ms: 420_000,
            threshold_minutes: 5,
          },
        ],
      },
    );
    expect(activity.hits.map(({ flag }) => flag.name)).toEqual(["Run duration", "Timeout"]);
    expect(activity.hits[0].rule?.duration_ms).toBe(420_000);
    expect(activity.hits[0].flag.evidence?.span_id).toBe("latest");
    expect(activity.traces).toBe(2);
    expect(activity.buckets[0].count).toBe(2);
  });
  it("counts alerts separately from traces and assigns hourly boundaries without double counting", () => {
    const activity = alertActivity(
      overview([
        {
          trace_id: "a",
          status: "classified",
          classified_at: new Date(since + HOUR_MS - 1).toISOString(),
          flags: [timeout, refusal],
        },
        {
          trace_id: "b",
          status: "classified",
          classified_at: new Date(since + HOUR_MS).toISOString(),
          flags: [timeout],
        },
        { trace_id: "missing", status: "classified", flags: [refusal] },
        {
          trace_id: "before",
          status: "classified",
          classified_at: new Date(since - 1).toISOString(),
          flags: [timeout],
        },
        {
          trace_id: "after",
          status: "classified",
          classified_at: new Date(since + 24 * HOUR_MS).toISOString(),
          flags: [timeout],
        },
      ]),
    );
    expect(activity.hits).toHaveLength(6);
    expect(activity.traces).toBe(5);
    expect(activity.signals.map(({ id, count }) => ({ id, count }))).toEqual([
      { id: "timeout", count: 4 },
      { id: "refusal", count: 2 },
    ]);
    expect(activity.buckets[0].count).toBe(2);
    expect(activity.buckets[1].count).toBe(1);
    expect(activity.buckets.slice(2).every((bucket) => bucket.count === 0)).toBe(true);
    expect(activity.unplotted).toBe(3);
    expect(activity.buckets.reduce((total, bucket) => total + bucket.count, activity.unplotted)).toBe(
      activity.hits.length,
    );
    expect(
      activity.buckets.every(
        (bucket) => bucket.signals.reduce((total, signal) => total + signal.count, 0) === bucket.count,
      ),
    ).toBe(true);
  });

  it("keeps traces with the same ID in different projects separate and ignores unflagged traces", () => {
    const activity = alertActivity(
      overview([
        {
          trace_id: "same",
          trace_ref: "one",
          status: "classified",
          flags: [timeout, refusal],
        },
        {
          trace_id: "same",
          trace_ref: "two",
          status: "classified",
          flags: [timeout],
        },
        { trace_id: "no-alert", status: "classified", flags: [] },
      ]),
    );
    expect(activity.traces).toBe(2);
    expect(activity.hits).toHaveLength(3);
    const empty = alertActivity(overview([]));
    expect(empty.signals).toEqual([]);
    expect(empty.unplotted).toBe(0);
    expect(empty.buckets.every((bucket) => bucket.count === 0)).toBe(true);
  });
});
