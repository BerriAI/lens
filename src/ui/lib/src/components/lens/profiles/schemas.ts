import { z } from "zod";

const money = z.number().finite().nullable();
const count = z.number().int().nonnegative();

export const profileSources = ["recorded", "local", "uncertain", "unclassified"] as const;

export const windowSchema = z.object({ agent: z.string(), start_ms: z.number(), end_ms: z.number() });
export const coverageSchema = z.object({ runs_listed: count, runs_resolved: count, truncated: z.boolean() });

export const profileSummarySchema = z.object({
  profile_id: z.string(),
  name: z.string(),
  source: z.enum(profileSources),
  run_count: count,
  share: z.number(),
  request_count: count,
  cost_observations: count,
  spend_usd: money,
});

export const overviewSchema = z.object({
  window: windowSchema,
  coverage: coverageSchema,
  totals: z.object({
    runs: count,
    requests: count,
    matched_requests: count,
    cost_observations: count,
    spend_usd: money,
    other_agent_requests: count,
    shared_requests: count,
  }),
  profiles: z.array(profileSummarySchema),
});

const capabilityFlags = z.object({
  function_calling: z.boolean().nullable(),
  vision: z.boolean().nullable(),
  reasoning: z.boolean().nullable(),
  structured_output: z.boolean().nullable(),
});

export const matrixCellSchema = z.object({
  profile_id: z.string(),
  column_key: z.string(),
  request_count: count,
  avg_cost_usd: money,
  cost_observations: count,
  p95_latency_ms: z.number().finite().nullable(),
  latency_observations: count,
  eval_success_rate: z.number().nullable(),
  eval_sample_count: count,
});

export const matrixSchema = z.object({
  window: windowSchema,
  coverage: coverageSchema,
  profiles: z.array(
    z.object({
      profile_id: z.string(),
      name: z.string(),
      source: z.enum(profileSources),
    }),
  ),
  models: z.array(
    z.object({
      column_key: z.string(),
      model_config_id: z.string().nullable(),
      model_name: z.string(),
      model_group: z.string(),
      provider: z.string(),
      configured: z.boolean(),
      capabilities: capabilityFlags,
    }),
  ),
  cells: z.array(matrixCellSchema),
  other_agent_requests: count,
});

export const detailSchema = z.object({
  window: windowSchema,
  coverage: coverageSchema,
  profile: profileSummarySchema,
  runs: z.array(
    z.object({
      trace_id: z.string(),
      trace_ref: z.string(),
      start_ms: z.number(),
      input_preview: z.string(),
      request_count: count,
      spend_usd: money,
    }),
  ),
  models: z.array(matrixCellSchema),
});

export type ProfileSource = (typeof profileSources)[number];
export type ProfileSummary = z.infer<typeof profileSummarySchema>;
export type ProfilesOverview = z.infer<typeof overviewSchema>;
export type ProfilesMatrix = z.infer<typeof matrixSchema>;
export type MatrixCell = z.infer<typeof matrixCellSchema>;
export type ProfileDetail = z.infer<typeof detailSchema>;
