import { z } from "zod";

const money = z.number().finite().nullable();
const count = z.number().int().nonnegative();

export const profileSources = ["recorded", "enhanced", "pending", "unknown", "local", "uncertain", "unclassified"] as const;
export const capabilities = ["function_calling", "vision", "reasoning", "structured_output"] as const;

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
  center: z.object({ x: z.number(), y: z.number() }),
});

export const taskDefinitionSchema = z.object({
  task_id: z.string(),
  name: z.string(),
  description: z.string(),
  inclusion_criteria: z.string(),
  exclusion_criteria: z.string(),
  required_capabilities: z.array(z.enum(capabilities)),
  example_trace_ids: z.array(z.string()),
});

const taxonomyStatus = z.enum(["discovering", "draft", "frozen", "failed", "cleared"]);

export const taxonomySummarySchema = z.object({
  taxonomy_id: z.string(),
  version: count,
  status: taxonomyStatus,
  sample: z.boolean(),
  task_count: count,
  frozen_at: z.string().nullable(),
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
  points: z.array(
    z.object({
      trace_id: z.string(),
      profile_id: z.string(),
      x: z.number(),
      y: z.number(),
      placement: z.enum(["discovery", "categorical"]),
    }),
  ),
  taxonomy: taxonomySummarySchema.nullable(),
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
      required_capabilities: z.array(z.enum(capabilities)).nullable(),
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
  compatibility: z.array(
    z.object({
      profile_id: z.string(),
      column_key: z.string(),
      status: z.enum(["compatible", "incompatible", "unknown"]),
      missing: z.array(z.enum(capabilities)),
      unknown: z.array(z.enum(capabilities)),
    }),
  ),
  other_agent_requests: count,
});

export const detailSchema = z.object({
  window: windowSchema,
  coverage: coverageSchema,
  profile: profileSummarySchema,
  definition: taskDefinitionSchema.nullable(),
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

export const taxonomySchema = z.object({
  taxonomy_id: z.string(),
  agent: z.string(),
  version: count,
  revision: count,
  status: taxonomyStatus,
  sample: z.boolean(),
  tasks: z.array(taskDefinitionSchema),
  points: z.array(
    z.object({
      team_id: z.string(),
      api_key_hash: z.string(),
      trace_id: z.string(),
      task_id: z.string().nullable(),
      x: z.number(),
      y: z.number(),
    }),
  ),
  job: z
    .object({
      job_id: z.string(),
      requested_at: z.string(),
      lease_until: z.string().nullable(),
      attempts: count,
      max_runs: count,
      start_ms: z.number(),
      end_ms: z.number(),
    })
    .nullable(),
  discovered_at: z.string().nullable(),
  frozen_at: z.string().nullable(),
  error: z.string().nullable(),
  models: z.object({ distill: z.string(), embedding: z.string(), synthesis: z.string(), decision: z.string() }),
  discovery: z
    .object({ start_ms: z.number(), end_ms: z.number(), requested_runs: count, sampled_runs: count })
    .nullable(),
  windows: z.array(z.object({ start_ms: z.number(), end_ms: z.number().nullable() })),
});

export const taxonomyViewSchema = z.object({
  taxonomy: taxonomySchema.nullable(),
  active: taxonomySchema.nullable(),
  discovery_available: z.boolean(),
  can_write: z.boolean(),
});

const modelChoiceSchema = z.object({ discovery: z.string(), embedding: z.string(), classification: z.string() });

export const settingsSchema = z.object({
  agent: z.string(),
  enabled: z.boolean(),
  models: modelChoiceSchema,
  revision: count,
  updated_at: z.string().nullable(),
});

export const settingsViewSchema = z.object({
  settings: settingsSchema,
  defaults: modelChoiceSchema,
  options: z.object({
    discovery: z.array(z.string()),
    embedding: z.array(z.string()),
    classification: z.array(z.string()),
  }),
  can_write: z.boolean(),
});

export const previewSchema = z.object({
  eligible_runs: count,
  sample_runs: count,
  previews: z.array(z.object({ trace_id: z.string(), start_ms: z.number(), input_preview: z.string() })),
});

export type Capability = (typeof capabilities)[number];
export type ProfileSource = (typeof profileSources)[number];
export type ProfileSummary = z.infer<typeof profileSummarySchema>;
export type ProfilesOverview = z.infer<typeof overviewSchema>;
export type ProfilesMatrix = z.infer<typeof matrixSchema>;
export type MatrixCell = z.infer<typeof matrixCellSchema>;
export type ProfileDetail = z.infer<typeof detailSchema>;
export type TaskDefinition = z.infer<typeof taskDefinitionSchema>;
export type Taxonomy = z.infer<typeof taxonomySchema>;
export type TaxonomyView = z.infer<typeof taxonomyViewSchema>;
export type ModelChoice = z.infer<typeof modelChoiceSchema>;
export type ProfileSettings = z.infer<typeof settingsSchema>;
export type ProfileSettingsView = z.infer<typeof settingsViewSchema>;
export type DiscoveryPreview = z.infer<typeof previewSchema>;
export type TaskEdit = Pick<TaskDefinition, "name" | "description" | "inclusion_criteria" | "exclusion_criteria" | "required_capabilities">;
