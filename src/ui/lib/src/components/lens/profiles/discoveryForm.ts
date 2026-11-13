import type { TimeWindow } from "../../shared/timeRange/timeRange";
import type { DiscoveryPreview, ModelChoice, ProfileSettings, Taxonomy } from "./schemas";

export const DEFAULT_SAMPLE = 300;
export const MIN_SAMPLE = 20;
export const MAX_SAMPLE = 500;

const pad = (value: number) => String(value).padStart(2, "0");

export const toDateInput = (ms: number) => {
  const date = new Date(ms);
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
};

const localMidnight = (value: string, dayOffset = 0) => {
  const [year, month, day] = value.split("-").map(Number);
  if (!year || !month || !day) return null;
  const date = new Date(year, month - 1, day + dayOffset);
  return Number.isNaN(date.getTime()) ? null : date.getTime();
};

/** Both dates are inclusive whole days in the browser's time zone, cut off at the present. */
export function rangeFromDates(start: string, end: string, nowMs: number): TimeWindow | null {
  const from = localMidnight(start);
  const after = localMidnight(end, 1);
  if (from === null || after === null) return null;
  const endMs = Math.min(after, nowMs);
  return endMs > from ? { startMs: from, endMs } : null;
}

export function sampleSize(input: string): number | null {
  const value = Number(input);
  return Number.isInteger(value) && value >= MIN_SAMPLE && value <= MAX_SAMPLE ? value : null;
}

export const sameModels = (a: ModelChoice, b: ModelChoice) =>
  a.discovery === b.discovery && a.embedding === b.embedding && a.classification === b.classification;

export const completeModels = (models: ModelChoice) =>
  [models.discovery, models.embedding, models.classification].every((alias) => alias.trim() !== "");

export function initialModels(saved: ProfileSettings, defaults: ModelChoice): ModelChoice {
  return {
    discovery: saved.models.discovery || defaults.discovery,
    embedding: saved.models.embedding || defaults.embedding,
    classification: saved.models.classification || defaults.classification,
  };
}

export type DiscoveryBlocker =
  | "disabled"
  | "unsaved"
  | "range"
  | "size"
  | "previewing"
  | "too-few"
  | "running";

export function discoveryBlocker(input: {
  saved: ProfileSettings;
  dirty: boolean;
  range: TimeWindow | null;
  size: number | null;
  preview: DiscoveryPreview | undefined;
  running: boolean;
}): DiscoveryBlocker | null {
  if (!input.saved.enabled) return "disabled";
  if (input.dirty) return "unsaved";
  if (input.running) return "running";
  if (!input.range) return "range";
  if (input.size === null) return "size";
  if (!input.preview) return "previewing";
  if (input.preview.sample_runs < MIN_SAMPLE) return "too-few";
  return null;
}

export const BLOCKER_TEXT: Record<DiscoveryBlocker, string> = {
  disabled: "Turn on task discovery and save to start a discovery.",
  unsaved: "Save your changes before starting a discovery.",
  range: "Choose a start date on or before the end date.",
  size: `Sample between ${MIN_SAMPLE} and ${MAX_SAMPLE} runs.`,
  previewing: "Checking which runs discovery would read…",
  "too-few": `Discovery needs at least ${MIN_SAMPLE} runs with a request in this range.`,
  running: "Discovery is already running for this agent.",
};

const formatDay = (ms: number) => new Date(ms).toLocaleDateString(undefined, { dateStyle: "medium" });
const formatMoment = (ms: number) => new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });

export const formatRange = (startMs: number, endMs: number) => `${formatDay(startMs)} to ${formatDay(endMs - 1)}`;

export type Classification =
  | { kind: "sample" }
  | { kind: "running"; since: number; backfill: { startMs: number; endMs: number } | null }
  | { kind: "paused" };

export function classification(active: Taxonomy): Classification {
  if (active.sample) return { kind: "sample" };
  const open = active.windows.find((window) => window.end_ms === null);
  if (!open) return { kind: "paused" };
  const backfill = active.discovery
    ? active.windows.find(
        (window) => window.start_ms === active.discovery?.start_ms && window.end_ms === active.discovery.end_ms,
      )
    : undefined;
  return {
    kind: "running",
    since: open.start_ms,
    backfill: backfill?.end_ms ? { startMs: backfill.start_ms, endMs: backfill.end_ms } : null,
  };
}

export function classificationText(state: Classification): string {
  switch (state.kind) {
    case "sample":
      return "Sample tasks with precomputed assignments. They never make model calls.";
    case "paused":
      return "Classification is paused. New runs keep their automatic groups until you resume it.";
    case "running":
      return state.backfill
        ? `Classifying runs that started since ${formatMoment(state.since)}, plus ${formatRange(state.backfill.startMs, state.backfill.endMs)}.`
        : `Classifying runs that started since ${formatMoment(state.since)}.`;
  }
}
