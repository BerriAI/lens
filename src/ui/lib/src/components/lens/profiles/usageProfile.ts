import { z } from "zod";
import type { TimeWindow } from "../../shared/timeRange/timeRange";

export const USAGE_CATEGORY_KEYS = ["task_category", "scenario"] as const;

const categorySchema = z.object({
  category: z.string(),
  runs: z
    .union([z.number(), z.string().regex(/^\d+$/)])
    .transform(Number)
    .pipe(z.number().int().nonnegative()),
});
export const usageProfileSchema = z.object({ data: z.array(categorySchema) });
export type UsageCategory = z.infer<typeof categorySchema>;

const sqlString = (value: string): string =>
  `'${value.replaceAll("\\", "\\\\").replaceAll("'", "\\'")}'`;

export function usageProfileSql(window: TimeWindow, agent: string): string {
  const recorded = USAGE_CATEGORY_KEYS.flatMap((key) => [
    `nullIf(trimBoth(SpanAttributes[${sqlString(key)}]), '')`,
    `nullIf(trimBoth(ResourceAttributes[${sqlString(key)}]), '')`,
  ]).join(", ");
  return `WITH runs AS (
  SELECT TeamId, ApiKeyHash, TraceId
  FROM agent_traces_by_key
  GROUP BY TeamId, ApiKeyHash, TraceId
  HAVING min(StartTs) >= fromUnixTimestamp64Milli(${Math.trunc(window.startMs)})
     AND min(StartTs) < fromUnixTimestamp64Milli(${Math.trunc(window.endMs)})
), recorded AS (
  SELECT TeamId, ApiKeyHash, TraceId, AgentName, ParentSpanId, Timestamp, SpanId,
         coalesce(${recorded}, '') AS recorded_category
  FROM otel_traces
  WHERE (TeamId, ApiKeyHash, TraceId) IN (SELECT TeamId, ApiKeyHash, TraceId FROM runs)
), categories AS (
  SELECT TeamId, ApiKeyHash, TraceId,
         argMinIf(recorded_category, tuple(if(ParentSpanId = '', 0, 1), Timestamp, SpanId), recorded_category != '') AS task_category,
         groupUniqArrayIf(AgentName, AgentName != '') AS agents
  FROM recorded
  GROUP BY TeamId, ApiKeyHash, TraceId
)
SELECT coalesce(nullIf(categories.task_category, ''), nullIf(inferred.Category, ''), 'Unclassified') AS category, count() AS runs
FROM runs LEFT JOIN categories USING (TeamId, ApiKeyHash, TraceId)
LEFT JOIN (
  SELECT TeamId, ApiKeyHash, TraceId, argMax(Category, ComputedAt) AS Category
  FROM lens_task_profiles
  WHERE ResolutionVersion = 1
    AND (TeamId, ApiKeyHash, TraceId) IN (SELECT TeamId, ApiKeyHash, TraceId FROM runs)
  GROUP BY TeamId, ApiKeyHash, TraceId
) AS inferred USING (TeamId, ApiKeyHash, TraceId)
WHERE ${sqlString(agent)} = '' OR has(categories.agents, ${sqlString(agent)})
GROUP BY category
ORDER BY runs DESC, category`;
}

export function usageCategory(
  attributes: Readonly<Record<string, string>>,
): string | null {
  return (
    USAGE_CATEGORY_KEYS.map((key) => attributes[key]?.trim()).find(Boolean) ??
    null
  );
}

export function summarizeUsage(categories: readonly UsageCategory[]) {
  const total = categories.reduce((sum, category) => sum + category.runs, 0);
  const classified = categories.reduce(
    (sum, category) =>
      sum + (category.category === "Unclassified" ? 0 : category.runs),
    0,
  );
  return {
    total,
    classified,
    categories: categories.filter((category) => category.runs > 0),
  };
}
