import type { MatrixCell, ProfilesMatrix, ProfilesOverview, ProfileSource, ProfileSummary } from "./schemas";

export const HEAT_METRICS = ["requests", "cost", "latency"] as const;
export type HeatMetric = (typeof HEAT_METRICS)[number];

export const RANKS = ["runs", "spend"] as const;
export type Rank = (typeof RANKS)[number];

const OUTSIDE_GROUPS: readonly ProfileSource[] = ["pending", "unknown", "uncertain", "unclassified"];

export const SOURCE_LABELS: Record<ProfileSource, string> = {
  recorded: "Recorded category",
  enhanced: "Discovered category",
  pending: "Awaiting classification",
  unknown: "Fits no category",
  local: "Automatic group",
  uncertain: "Automatic grouping unsure",
  unclassified: "Not grouped yet",
};

export const isOutsideGroups = (source: ProfileSource) => OUTSIDE_GROUPS.includes(source);

const TOKENS = [1, 2, 3, 4, 5].map((index) => `var(--chart-${index})`);
const PALETTE = [...TOKENS, ...TOKENS.map((token) => `color-mix(in oklab, ${token} 62%, var(--foreground))`)];
export const NEUTRAL = "var(--muted-foreground)";

const hash = (text: string) => [...text].reduce((value, char) => Math.imul(value ^ (char.codePointAt(0) ?? 0), 16777619) >>> 0, 2166136261);

export const profileColor = (profile: Pick<ProfileSummary, "profile_id" | "source">) =>
  isOutsideGroups(profile.source) ? NEUTRAL : PALETTE[hash(profile.profile_id) % PALETTE.length];

export function profileColors(profiles: readonly Pick<ProfileSummary, "profile_id" | "source">[]): ReadonlyMap<string, string> {
  return new Map(profiles.map((profile) => [profile.profile_id, profileColor(profile)]));
}

export interface RankedProfile {
  profile: ProfileSummary;
  value: number | null;
  fraction: number;
}

export function rankProfiles(profiles: readonly ProfileSummary[], rank: Rank): RankedProfile[] {
  const valued = profiles.map((profile) => ({
    profile,
    value: rank === "runs" ? profile.run_count : profile.spend_usd,
  }));
  const top = Math.max(0, ...valued.map((entry) => entry.value ?? 0));
  return valued
    .map((entry) => ({ ...entry, fraction: top > 0 && entry.value !== null ? entry.value / top : 0 }))
    .sort(
      (a, b) =>
        Number(a.value === null) - Number(b.value === null) ||
        (b.value ?? 0) - (a.value ?? 0) ||
        a.profile.name.localeCompare(b.profile.name),
    );
}

export interface SimilarityGroup {
  profile: ProfileSummary;
  points: { key: string; x: number; y: number }[];
}

export interface MapCoverage {
  profile: ProfileSummary;
  runs: number;
  shown: number;
  mappable: boolean;
}

export interface Similarity {
  groups: SimilarityGroup[];
  coverage: MapCoverage[];
  placed: number;
  total: number;
}

export function similarity(overview: ProfilesOverview): Similarity {
  const mappable = new Set(
    overview.profiles.filter((profile) => !isOutsideGroups(profile.source)).map((profile) => profile.profile_id),
  );
  const genuine = overview.points
    .map(({ profile_id, trace_id, x, y }, index) => ({ key: `${index}:${trace_id}`, profile_id, x, y }))
    .filter((point) => mappable.has(point.profile_id));
  const groups = overview.profiles
    .map((profile) => ({
      profile,
      points: genuine.filter((point) => point.profile_id === profile.profile_id).map(({ key, x, y }) => ({ key, x, y })),
    }))
    .filter((group) => group.points.length > 0);
  const shownBy = new Map(groups.map((group) => [group.profile.profile_id, group.points.length]));
  return {
    groups,
    coverage: overview.profiles
      .map((profile) => ({
        profile,
        runs: profile.run_count,
        shown: shownBy.get(profile.profile_id) ?? 0,
        mappable: mappable.has(profile.profile_id),
      }))
      .sort((a, b) => b.runs - a.runs || a.profile.name.localeCompare(b.profile.name)),
    placed: genuine.length,
    total: overview.totals.runs,
  };
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

export interface ModelLabel {
  name: string;
  detail: string | null;
}

export function modelLabel(columnKey: string, models: readonly ProfilesMatrix["models"][number][]): ModelLabel {
  const model = models.find((entry) => entry.column_key === columnKey);
  const fallback = columnKey.replace(/^(id|group|unmatched):/, "");
  if (!model) return { name: fallback, detail: null };
  return { name: model.model_name || model.model_group || fallback, detail: model.model_config_id };
}
