import type { Finding, Sample } from "./types";

type Execution = Sample["executions"][number];

export interface DayCount {
  readonly day: string;
  readonly affected: number;
  readonly unaffected: number;
}

export interface Frequency {
  readonly affected: number;
  readonly total: number | null;
  readonly days: readonly DayCount[];
}

const DAY_MS = 86_400_000;
export const FREQUENCY_WINDOW_DAYS = 14;

const dayKey = (ms: number): string => new Date(ms).toISOString().slice(0, 10);

function dayRange(first: number, last: number): string[] {
  const end = Date.parse(dayKey(last));
  const start = Math.min(Date.parse(dayKey(first)), end - (FREQUENCY_WINDOW_DAYS - 1) * DAY_MS);
  const span = Math.round((end - start) / DAY_MS);
  return Array.from({ length: span + 1 }, (_, i) => dayKey(start + i * DAY_MS));
}

/**
 * How many sampled traces a finding hit, overall and per UTC day. Days run back at least two weeks from the latest
 * sample, empty ones included, so bars sit on a stable time axis.
 */
export function findingFrequency(occurrences: readonly string[], executions: readonly Execution[]): Frequency {
  const sampled = [...new Map(executions.map((run) => [run.id, run] as const)).values()];
  const hit = new Set(occurrences);
  const dated = sampled
    .map((run) => ({
      affected: hit.has(run.id),
      ms: Date.parse(run.start_time),
    }))
    .filter(({ ms }) => Number.isFinite(ms));
  const stamps = dated.map(({ ms }) => ms);
  const days = dated.length ? dayRange(Math.min(...stamps), Math.max(...stamps)) : [];
  return {
    affected: sampled.filter((run) => hit.has(run.id)).length,
    total: sampled.length,
    days: days.map((day) => {
      const onDay = dated.filter(({ ms }) => dayKey(ms) === day);
      const affected = onDay.filter((run) => run.affected).length;
      return { day, affected, unaffected: onDay.length - affected };
    }),
  };
}

type FindingIdentity = Pick<Finding, "id" | "existing_finding_id" | "merged_finding_ids">;

export function hasImportedEvidence(finding: FindingIdentity): boolean {
  return [finding.id, finding.existing_finding_id, ...(finding.merged_finding_ids ?? [])].some(
    (id) => id != null && /^agent-[a-f0-9]{64}$/.test(id),
  );
}

export function findingTraceFrequency(
  finding: FindingIdentity & Pick<Finding, "occurrences">,
  executions: readonly Execution[],
  hasUnscopedEvidence = hasImportedEvidence(finding),
): Frequency {
  if (hasUnscopedEvidence) return { affected: new Set(finding.occurrences).size, total: null, days: [] };
  return findingFrequency(finding.occurrences, executions);
}

export function percentLabel(affected: number, total: number | null): string | null {
  if (total === null || total <= 0) return null;
  return `${Number(((affected / total) * 100).toFixed(1))}%`;
}

export const dayLabel = (day: string): string =>
  new Date(`${day}T00:00:00Z`).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    timeZone: "UTC",
  });
