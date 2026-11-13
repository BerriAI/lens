import { afterEach, describe, expect, it, vi } from "vitest";
import {
  classification,
  discoveryBlocker,
  initialModels,
  rangeFromDates,
  sampleSize,
} from "./discoveryForm";
import type { DiscoveryPreview, ProfileSettings, Taxonomy } from "./schemas";

const settings = (overrides: Partial<ProfileSettings> = {}): ProfileSettings => ({
  agent: "assistant",
  enabled: true,
  models: { discovery: "haiku", embedding: "qwen", classification: "jev" },
  revision: 2,
  updated_at: null,
  ...overrides,
});

const preview = (sample_runs: number): DiscoveryPreview => ({ eligible_runs: sample_runs, sample_runs, previews: [] });

const ready = {
  saved: settings(),
  dirty: false,
  range: { startMs: 0, endMs: 1 },
  size: 300,
  preview: preview(300),
  running: false,
};

const active = (overrides: Partial<Taxonomy>): Taxonomy => ({
  taxonomy_id: "tax",
  agent: "assistant",
  version: 1,
  revision: 1,
  status: "frozen",
  tasks: [],
  points: [],
  job: null,
  discovered_at: null,
  frozen_at: null,
  error: null,
  models: { distill: "haiku", embedding: "qwen", synthesis: "haiku", decision: "jev" },
  discovery: { start_ms: 100, end_ms: 200, requested_runs: 300, sampled_runs: 50 },
  windows: [],
  ...overrides,
});

afterEach(() => vi.unstubAllEnvs());

describe("rangeFromDates", () => {
  it("covers both chosen days in full", () => {
    const range = rangeFromDates("2026-11-01", "2026-11-02", Number.MAX_SAFE_INTEGER);

    expect(range).toEqual({
      startMs: new Date(2026, 10, 1).getTime(),
      endMs: new Date(2026, 10, 3).getTime(),
    });
  });

  it("stops at the present instead of reaching into the future", () => {
    const now = new Date(2026, 10, 2, 9, 30).getTime();

    expect(rangeFromDates("2026-11-01", "2026-11-02", now)?.endMs).toBe(now);
  });

  it.each([
    ["reversed", "2026-11-03", "2026-11-02"],
    ["incomplete", "", "2026-11-02"],
  ])("rejects a %s range", (_, start, end) => {
    expect(rangeFromDates(start, end, Number.MAX_SAFE_INTEGER)).toBeNull();
  });

  it("rejects a range that starts in the future", () => {
    const now = new Date(2026, 10, 13, 12).getTime();

    expect(rangeFromDates("2026-11-20", "2026-11-21", now)).toBeNull();
  });

  it("ends a daylight saving day at the next calendar midnight, not after 24 hours", () => {
    vi.stubEnv("TZ", "America/New_York");

    const range = rangeFromDates("2026-03-08", "2026-03-08", Number.MAX_SAFE_INTEGER);

    expect(range).toEqual({ startMs: new Date(2026, 2, 8).getTime(), endMs: new Date(2026, 2, 9).getTime() });
    expect((range?.endMs ?? 0) - (range?.startMs ?? 0)).toBe(23 * 60 * 60 * 1000);
  });
});

describe("sampleSize", () => {
  it.each([
    ["19", null],
    ["20", 20],
    ["500", 500],
    ["501", null],
    ["12.5", null],
  ])("reads %s as %s", (input, expected) => {
    expect(sampleSize(input)).toBe(expected);
  });
});

describe("discoveryBlocker", () => {
  it.each([
    ["off", { saved: settings({ enabled: false }), dirty: true }, "disabled"],
    ["unsaved", { dirty: true }, "unsaved"],
    ["running", { running: true }, "running"],
    ["no range", { range: null }, "range"],
    ["bad size", { size: null }, "size"],
    ["no preview yet", { preview: undefined }, "previewing"],
    ["too few runs", { preview: preview(19) }, "too-few"],
    ["ready", {}, null],
  ] as const)("reports %s", (_, overrides, expected) => {
    expect(discoveryBlocker({ ...ready, ...overrides })).toBe(expected);
  });
});

describe("initialModels", () => {
  it("prefers saved aliases and fills only empty roles from the deployment defaults", () => {
    const saved = settings({ models: { discovery: "sonnet", embedding: "", classification: "" } });

    expect(initialModels(saved, { discovery: "haiku", embedding: "qwen", classification: "jev" })).toEqual({
      discovery: "sonnet",
      embedding: "qwen",
      classification: "jev",
    });
  });
});

describe("classification", () => {
  it("reports paused when every window is closed", () => {
    expect(classification(active({ windows: [{ start_ms: 300, end_ms: 400 }] }))).toEqual({ kind: "paused" });
  });

  it("reports the open window and the backfilled discovery range", () => {
    const state = classification(
      active({
        windows: [
          { start_ms: 100, end_ms: 200 },
          { start_ms: 500, end_ms: null },
        ],
      }),
    );

    expect(state).toEqual({ kind: "running", since: 500, backfill: { startMs: 100, endMs: 200 } });
  });
});
