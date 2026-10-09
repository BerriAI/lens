/* Generated from schema/lens-worker.v7.json. Run npm run generate:worker-contract. */

/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "TraceSignalStatus".
 */
export type TraceSignalStatus = "unclassified" | "pending" | "classified" | "failed";

export interface LensWorker {
  [k: string]: unknown;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "Activity".
 */
export interface Activity {
  execution_ids?: string[];
  finished?: boolean;
  id: string;
  label: string;
  operations?: (
    | "model"
    | "read"
    | "search"
    | "python"
    | "catalog"
    | "review_catalog"
    | "read_reviews"
    | "search_reviews"
    | "history"
    | "checkpoint"
  )[];
  phase: "load" | "review" | "group" | "reconcile" | "investigate";
  started_at: string;
  tool_calls?: ToolCount[];
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "ToolCount".
 */
export interface ToolCount {
  calls: number;
  name:
    | "model"
    | "read"
    | "search"
    | "python"
    | "catalog"
    | "review_catalog"
    | "read_reviews"
    | "search_reviews"
    | "history"
    | "checkpoint";
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "AgentTestCase".
 */
export interface AgentTestCase {
  expected: string;
  input: string;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "Candidate".
 */
export interface Candidate {
  check_id: string;
  execution_ids: string[];
  existing_finding_id?: string | null;
  hypothesis: string;
  kind?: "issue" | "pattern";
  title: string;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "CatalogEntry".
 */
export interface CatalogEntry {
  characters: number | null;
  execution: Execution;
  partial: boolean;
  spans: [string, string, string, string, number | null, string, string][];
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "Execution".
 */
export interface Execution {
  id: string;
  metadata?: MetadataFilter[];
  name: string;
  root_seen?: boolean;
  service?: string;
  source: "traces" | "requests";
  span_count: number;
  start_time: string;
  team_id: string;
  trace_id: string;
  trace_ref?: string;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "MetadataFilter".
 */
export interface MetadataFilter {
  key: string;
  value: string;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "Check".
 */
export interface Check {
  enabled?: boolean;
  id: string;
  instruction: string;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "Checkpoint".
 */
export interface Checkpoint {
  working_notes: string;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "Claim".
 */
export interface Claim {
  findings: Finding[];
  job: Job;
  lens_id: string;
  reviews?: Review[] | null;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "Finding".
 */
export interface Finding {
  brief?: IssueBrief | null;
  check_id: string;
  check_ids?: string[];
  description: string;
  /**
   * @minItems 1
   */
  evidence: Evidence[];
  existing_finding_id?: string | null;
  first_seen: string;
  id: string;
  investigation_runs?: string[];
  kind?: "issue" | "pattern";
  last_seen: string;
  limitation?: string;
  merged_finding_ids?: string[];
  occurrences?: string[];
  priority?: "high" | "medium" | "low";
  reason?: string;
  revision: number;
  status?: "open" | "resolved" | "dismissed";
  suggestion?: string;
  title: string;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "IssueBrief".
 */
export interface IssueBrief {
  problem: string;
  /**
   * @minItems 1
   */
  test_cases: AgentTestCase[];
  user_goal: string;
  what_happened: string;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "Evidence".
 */
export interface Evidence {
  execution_id: string;
  quote: string;
  role?: "support" | "counterexample";
  span_id: string;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "Job".
 */
export interface Job {
  activities?: Activity[];
  assessments?: RunAssessment[];
  attempts?: number;
  cost?: number;
  coverage?: Coverage;
  created_at: string;
  end: string;
  error?: string;
  findings?: Finding[] | null;
  finished_at?: string | null;
  id: string;
  lease_until?: string | null;
  reading?: InFlight[];
  review_versions?: ReviewVersion[];
  reviewed?: number;
  reviews?: Review[];
  revision: number;
  sample?: Sample | null;
  settings: LensSettings;
  stage?: string;
  start: string;
  status?: "queued" | "running" | "completed" | "failed" | "cancelled";
  steps?: Step[];
  trigger?: "schedule" | "manual";
  worker_id?: string | null;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "RunAssessment".
 */
export interface RunAssessment {
  cannot_assess?: boolean;
  execution_id: string;
  issue_checks?: string[];
  pattern_checks?: string[];
}
export interface Coverage {
  candidates?: number;
  eligible?: number;
  failed_tasks?: number;
  grouped_batches?: number;
  grouping_batches?: number;
  inconclusive?: number;
  investigated?: number;
  partial?: number;
  reusable?: number;
  reused?: number;
  screened?: number;
  selected?: number;
  unassessable?: number;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "InFlight".
 */
export interface InFlight {
  agent: string;
  execution_id: string;
  started_at: string;
  trace_id: string;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "ReviewVersion".
 */
export interface ReviewVersion {
  content_version: string;
  execution_id: string;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "Review".
 */
export interface Review {
  agent: string;
  at: string;
  cannot_assess?: boolean;
  consolidated?: boolean;
  content_version?: string;
  duration_ms: number;
  execution_id: string;
  extraction?: Extraction | null;
  model: string;
  name: string;
  partial?: boolean;
  reasoning?: string;
  reused?: boolean;
  /**
   * @maxItems 8
   */
  spans?: ReviewSpan[];
  tool_calls?: ToolCount[];
  trace_id: string;
  verdicts?: ReviewVerdict[];
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "Extraction".
 */
export interface Extraction {
  cannot_assess?: boolean;
  observations?: Observation[];
  reasoning?: string;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "Observation".
 */
export interface Observation {
  check_id: string;
  evidence?: Evidence[];
  kind?: "issue" | "pattern";
  summary: string;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "ReviewSpan".
 */
export interface ReviewSpan {
  cited?: boolean;
  kind: string;
  name: string;
  preview: string;
  span_id: string;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "ReviewVerdict".
 */
export interface ReviewVerdict {
  check_id: string;
  kind: "issue" | "pattern";
  summary: string;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "Sample".
 */
export interface Sample {
  eligible: number;
  executions: Execution[];
  next_cursor?: string | null;
  next_offset?: number | null;
  selected?: number;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "LensSettings".
 */
export interface LensSettings {
  agent_name?: string;
  checks?: Check[];
  concurrency?: number;
  context?: string;
  enabled?: boolean;
  execution_ids?: string[];
  filters?: MetadataFilter[];
  interval_minutes?: number;
  lookback_hours?: number;
  model: string;
  monthly_budget?: number;
  name: string;
  sample_percent?: number;
  sample_size?: number | null;
  service?: string;
  source?: "traces" | "requests" | "both";
  team_id?: string;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "Step".
 */
export interface Step {
  at: string;
  completion_tokens?: number;
  cost?: number;
  kind: "stage" | "model" | "error";
  label: string;
  model?: string;
  prompt_tokens?: number;
  purpose?: string;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "Clusters".
 */
export interface Clusters {
  candidates?: Candidate[];
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "Coverage".
 */
export interface Coverage1 {
  candidates?: number;
  eligible?: number;
  failed_tasks?: number;
  grouped_batches?: number;
  grouping_batches?: number;
  inconclusive?: number;
  investigated?: number;
  partial?: number;
  reusable?: number;
  reused?: number;
  screened?: number;
  selected?: number;
  unassessable?: number;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "EvidenceReply".
 */
export interface EvidenceReply {
  catalog?: CatalogEntry[];
  error?: string;
  parts?: TracePart[];
  request: EvidenceRequest;
  review_catalog?: ReviewIndex[];
  reviews?: ReviewRecord[];
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "TracePart".
 */
export interface TracePart {
  content: string;
  end_time?: string;
  execution_id: string;
  kind: string;
  name: string;
  parent_span_id?: string;
  span_id: string;
  start_time?: string;
  truncated?: boolean;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "EvidenceRequest".
 */
export interface EvidenceRequest {
  action: "catalog" | "read" | "search" | "review_catalog" | "read_reviews" | "search_reviews" | "history";
  char_end?: number | null;
  char_start?: number;
  execution_id?: string | null;
  include_initial?: boolean;
  query?: string;
  review_phase?: ("initial" | "revisited") | null;
  span_ids?: string[];
  turn_end?: number | null;
  turn_start?: number;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "ReviewIndex".
 */
export interface ReviewIndex {
  characters: number;
  execution_id: string;
  phase: "initial" | "revisited";
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "ReviewRecord".
 */
export interface ReviewRecord {
  content: string;
  execution_id: string;
  phase: "initial" | "revisited";
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "ExecutionContent".
 */
export interface ExecutionContent {
  execution: Execution;
  next_cursor?: string | null;
  partial?: boolean;
  parts: TracePart[];
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "FindingDraft".
 */
export interface FindingDraft {
  brief?: IssueBrief | null;
  check_id: string;
  check_ids?: string[];
  description: string;
  /**
   * @minItems 1
   */
  evidence: Evidence[];
  existing_finding_id?: string | null;
  kind?: "issue" | "pattern";
  limitation?: string;
  merged_finding_ids?: string[];
  priority?: "high" | "medium" | "low";
  suggestion?: string;
  title: string;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "FindingGroup".
 */
export interface FindingGroup {
  /**
   * @minItems 1
   */
  members: string[];
  representative: string;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "FindingGroups".
 */
export interface FindingGroups {
  groups: FindingGroup[];
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "Findings".
 */
export interface Findings {
  findings?: FindingDraft[];
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "ModelMessage".
 */
export interface ModelMessage {
  content: string;
  role: "system" | "user" | "assistant";
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "ModelRequest".
 */
export interface ModelRequest {
  messages?: ModelMessage[];
  prompt: string;
  purpose: "extract" | "cluster" | "investigate";
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "ModelResult".
 */
export interface ModelResult {
  content: string;
  context_exceeded?: boolean;
  cost: number;
  finish_reason?: ("length" | "content_filter") | null;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "Progress".
 */
export interface Progress {
  activity?: Activity | null;
  coverage?: Coverage1 | null;
  reading?: InFlight[] | null;
  review?: Review | null;
  stage?: string | null;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "PythonAgentTurn[Extraction]".
 */
export interface PythonAgentTurnExtraction {
  checkpoint?: string | null;
  result?: Extraction | null;
  tools?: (EvidenceRequest | PythonRequest)[];
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "PythonRequest".
 */
export interface PythonRequest {
  action: "python";
  code: string;
  execution_ids?: string[];
  span_ids?: string[];
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "PythonAgentTurn[Findings]".
 */
export interface PythonAgentTurnFindings {
  checkpoint?: string | null;
  result?: Findings | null;
  tools?: (EvidenceRequest | PythonRequest)[];
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "Result".
 */
export interface Result {
  assessments?: RunAssessment[];
  coverage: Coverage1;
  error?: string;
  findings?: FindingDraft[];
  review_versions?: ReviewVersion[];
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "SignalEvidence".
 */
export interface SignalEvidence {
  quote: string;
  span_id: string;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "SignalFlag".
 */
export interface SignalFlag {
  evidence?: SignalEvidence | null;
  name: string;
  score: number;
  signal_id: string;
}
/**
 * This interface was referenced by `LensWorker`'s JSON-Schema
 * via the `definition` "TraceSignals".
 */
export interface TraceSignals {
  classified_at?: string | null;
  flags?: SignalFlag[];
  model?: string;
  status: TraceSignalStatus;
  trace_id: string;
  trace_ref?: string;
}
