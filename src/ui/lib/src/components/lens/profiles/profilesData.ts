import type { Capability, MatrixCell, ProfileSource, ProfileSummary } from "./schemas";

export const HEAT_METRICS = ["requests", "cost", "latency"] as const;
export type HeatMetric = (typeof HEAT_METRICS)[number];

const PALETTE = ["var(--chart-1)", "var(--chart-2)", "var(--chart-3)", "var(--chart-4)", "var(--chart-5)"];
export const MUTED = "var(--muted-foreground)";
const MUTED_SOURCES: readonly ProfileSource[] = ["pending", "unknown", "uncertain", "unclassified"];

export const SOURCE_LABELS: Record<ProfileSource, string> = {
  recorded: "Recorded category",
  enhanced: "Reviewed task",
  pending: "Awaiting classification",
  unknown: "Fits no reviewed task",
  local: "Automatic group",
  uncertain: "Automatic grouping unsure",
  unclassified: "Not grouped yet",
};

export function profileColors(profiles: readonly Pick<ProfileSummary, "profile_id" | "source">[]) {
  let next = 0;
  return new Map(
    profiles.map((profile) => [
      profile.profile_id,
      MUTED_SOURCES.includes(profile.source) ? MUTED : PALETTE[next++ % PALETTE.length],
    ]),
  );
}

/** Older links name a profile by its label; resolve them within the agent's current profiles. */
export function selectedProfileId(
  profiles: readonly Pick<ProfileSummary, "profile_id" | "name">[],
  param: string | null,
): string | null {
  if (!param) return null;
  return (
    profiles.find((profile) => profile.profile_id === param)?.profile_id ??
    profiles.find((profile) => profile.name === param)?.profile_id ??
    null
  );
}

export const formatCount = (value: number) => value.toLocaleString();

export const formatShare = (share: number) =>
  `${(share * 100).toLocaleString(undefined, { maximumFractionDigits: 1 })}%`;

export const formatUsd = (value: number | null): string | null => {
  if (value === null) return null;
  const digits = value !== 0 && Math.abs(value) < 0.01 ? 4 : 2;
  return `$${value.toLocaleString(undefined, { minimumFractionDigits: digits, maximumFractionDigits: digits })}`;
};

export const formatLatency = (value: number | null): string | null => {
  if (value === null) return null;
  return value >= 1000 ? `${(value / 1000).toLocaleString(undefined, { maximumFractionDigits: 1 })} s` : `${Math.round(value)} ms`;
};

export interface Reading {
  value: number | null;
  observations: number;
}

export function cellReading(cell: MatrixCell | undefined, metric: HeatMetric): Reading {
  if (!cell) return { value: null, observations: 0 };
  switch (metric) {
    case "requests":
      return { value: cell.request_count, observations: cell.request_count };
    case "cost":
      return { value: cell.avg_cost_usd, observations: cell.cost_observations };
    case "latency":
      return { value: cell.p95_latency_ms, observations: cell.latency_observations };
  }
}

export function formatReading(metric: HeatMetric, value: number | null): string | null {
  switch (metric) {
    case "requests":
      return value === null ? null : formatCount(value);
    case "cost":
      return formatUsd(value);
    case "latency":
      return formatLatency(value);
  }
}

/** 0 for the smallest observed value, 1 for the largest; missing values stay unshaded. */
export function heatIntensity(value: number | null, values: readonly (number | null)[]): number | null {
  if (value === null) return null;
  const present = values.filter((item): item is number => item !== null);
  const low = Math.min(...present);
  const high = Math.max(...present);
  return high > low ? (value - low) / (high - low) : 1;
}

const CAPABILITY_LABELS: Record<Capability, string> = {
  function_calling: "function calling",
  vision: "vision",
  reasoning: "reasoning",
  structured_output: "structured output",
};

export interface Fit {
  status: "compatible" | "incompatible" | "unknown";
  missing: readonly Capability[];
  unknown: readonly Capability[];
}

/** Compatibility compares declared metadata only; it says nothing about how well a model does the task. */
export function compatibilityNote(requirements: readonly Capability[] | null, fit: Fit | undefined): string {
  if (requirements === null) return "Requirements not reviewed";
  if (!fit) return "No metadata";
  const list = (capabilities: readonly Capability[]) => capabilities.map((capability) => CAPABILITY_LABELS[capability]).join(", ");
  switch (fit.status) {
    case "compatible":
      return requirements.length ? "Meets declared requirements" : "No special requirements";
    case "incompatible":
      return `Metadata says it lacks ${list(fit.missing)}`;
    case "unknown":
      return `${list(fit.unknown)} support unknown`;
  }
}
