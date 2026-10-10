import { randomUUID } from "node:crypto";
import { z } from "zod";
import type { AgentConfig as Config } from "./config.js";
import { LensClient } from "./lens.js";
import { ClickHouseState, digest, type StateStore } from "./state.js";
import { Evidence, type Sample } from "./evidence.js";
import { Repository } from "./repository.js";
import { investigate } from "./agent.js";
import {
  PROMPT_REVISION,
  type VerifiedCandidate,
  type PersistedCandidate,
} from "./findings.js";
import type { NativeFinding } from "./models.js";
export const provenance = z.object({
  model: z.string(),
  prompt_revision: z.string(),
  trace_ids: z.array(z.string()),
  repository_sha: z.string(),
  at: z.number(),
});
export const stateSchema = z.object({
  day: z.string(),
  next_at: z.number(),
  lease_until: z.number(),
  owner: z.string(),
  runs: z.number(),
  posts: z.number(),
  next_issue_number: z.number().int().positive().default(1),
  signature: z.string(),
  sent: z.array(
    z.object({
      fingerprint: z.string(),
      evidence_hash: z.string(),
      issue_key: z.string(),
      title: z.string(),
      issue_number: z.number().int().positive().optional(),
      native_finding: z
        .object({
          lensId: z.string(),
          findingId: z.string(),
          url: z.string().url(),
        })
        .optional(),
      provenance,
    }),
  ),
  last_run: provenance.nullable(),
});
export type State = z.infer<typeof stateSchema>;
export const initialState: State = {
  day: "",
  next_at: 0,
  lease_until: 0,
  owner: "",
  runs: 0,
  posts: 0,
  next_issue_number: 1,
  signature: "",
  sent: [],
  last_run: null,
};

export async function bootstrapKnownFindings(
  store: StateStore<State>,
  scope: readonly string[],
  raw?: string,
): Promise<void> {
  if (!raw) return;
  if (raw.length > 64_000) throw new Error("Known findings exceed limit");
  const seed = z
    .object({
      scope: z.array(z.string()).length(4),
      findings: stateSchema.shape.sent.min(1).max(10),
    })
    .parse(JSON.parse(raw));
  if (JSON.stringify(seed.scope) !== JSON.stringify(scope))
    throw new Error("Known findings scope does not match");
  const current = await store.read();
  if (current.revision !== 0) return;
  await store.commit(current, { ...current.value, sent: seed.findings });
  console.info("Lens investigator known reports initialized");
}
export interface InvestigatorDependencies {
  readonly store: StateStore<State>;
  readonly sample: (signal: AbortSignal) => Promise<Sample>;
  readonly analyze: (
    sample: Sample,
    previous: State["sent"],
    signal: AbortSignal,
  ) => Promise<{ candidate?: VerifiedCandidate; sha: string }>;
  readonly post: (
    candidate: PersistedCandidate,
    sample: Sample,
    source: z.infer<typeof provenance>,
  ) => Promise<void>;
  readonly persist: (
    candidate: VerifiedCandidate,
    sample: Sample,
    source: z.infer<typeof provenance>,
    signal: AbortSignal,
  ) => Promise<NativeFinding>;
  readonly model: string;
  readonly dailyLimit: number;
  readonly now?: () => number;
}

export async function cycle(
  deps: InvestigatorDependencies,
  signal: AbortSignal,
): Promise<void> {
  const now = deps.now ?? Date.now;
  const current = await deps.store.read();
  const started = now();
  if (current.value.next_at > started || current.value.lease_until > started)
    return;
  const day = new Date(started).toISOString().slice(0, 10);
  const previous =
    current.value.day === day
      ? current.value
      : { ...current.value, day, runs: 0, posts: 0 };
  const claimed = await deps.store.commit(current, {
    ...previous,
    next_at: started + 600_000,
    lease_until: started + 300_000,
    owner: randomUUID(),
  });
  console.info("Lens investigator durable lease acquired");
  if (claimed.value.runs >= deps.dailyLimit || claimed.value.posts >= 3) return;
  const sample = await deps.sample(signal);
  if (!sample.traces.length) return;
  const signature = digest(
    JSON.stringify(
      sample.traces
        .map((row) => [
          row.trace_id,
          row.status,
          row.duration_ms,
          row.error_count,
          row.llm_calls,
        ])
        .sort(),
    ),
  );
  if (signature === claimed.value.signature) return;
  if (now() >= claimed.value.lease_until - 30_000) return;
  const reserved = await deps.store.commit(claimed, {
    ...claimed.value,
    runs: claimed.value.runs + 1,
  });
  const result = await deps.analyze(sample, reserved.value.sent, signal);
  if (signal.aborted || now() >= reserved.value.lease_until - 30_000) return;
  const source = {
    model: deps.model,
    prompt_revision: PROMPT_REVISION,
    trace_ids: sample.traces.map((row) => row.trace_id),
    repository_sha: result.sha,
    at: started,
  };
  const candidate = result.candidate;
  if (!candidate) {
    await deps.store.commit(reserved, {
      ...reserved.value,
      last_run: source,
      signature,
      lease_until: 0,
    });
    console.info("Lens investigator analysis persisted; no new report");
    return;
  }
  const duplicate = reserved.value.sent.find(
    (item) =>
      item.fingerprint === candidate.fingerprint ||
      item.evidence_hash === candidate.evidenceHash ||
      item.title === candidate.candidate.title,
  );
  const persisted =
    duplicate && /^[a-f0-9]{64}$/.test(duplicate.fingerprint)
      ? { ...candidate, fingerprint: duplicate.fingerprint }
      : candidate;
  // Native import is idempotent. A timeout here must leave the evidence eligible
  // for retry, while an ambiguous Slack send below remains durably claimed.
  const native = await deps.persist(persisted, sample, source, signal);
  const owned = await deps.store.read();
  if (
    signal.aborted ||
    now() >= owned.value.lease_until - 30_000 ||
    owned.value.owner !== reserved.value.owner ||
    owned.revision !== reserved.revision
  )
    return;
  if (duplicate) {
    await deps.store.commit(owned, {
      ...owned.value,
      signature,
      last_run: source,
      lease_until: 0,
      sent: owned.value.sent.map((item) =>
        item.fingerprint === duplicate.fingerprint
          ? {
              ...item,
              fingerprint: persisted.fingerprint,
              evidence_hash: candidate.evidenceHash,
              provenance: source,
              native_finding: native,
            }
          : item,
      ),
    });
    console.info(
      "Lens investigator native finding refreshed; no new notification",
    );
    return;
  }
  const committed = await deps.store.commit(owned, {
    ...owned.value,
    signature,
    posts: reserved.value.posts + 1,
    next_issue_number: reserved.value.next_issue_number + 1,
    last_run: source,
    sent: [
      ...reserved.value.sent,
      {
        fingerprint: candidate.fingerprint,
        evidence_hash: candidate.evidenceHash,
        issue_key: candidate.candidate.issue_key,
        title: candidate.candidate.title,
        issue_number: reserved.value.next_issue_number,
        native_finding: native,
        provenance: source,
      },
    ].slice(-200),
  });
  if (now() >= committed.value.lease_until - 15_000 || signal.aborted) return;
  await deps.post(
    { ...candidate, native, issueNumber: reserved.value.next_issue_number },
    sample,
    source,
  );
  console.info("Lens investigator report delivered with durable provenance");
}

export interface InvestigatorOptions {
  readonly repository: string;
  readonly clientId: string;
  readonly key: string;
  readonly dailyLimit: number;
  readonly model: string;
  readonly scope: readonly string[];
  readonly stateConnection: string;
  readonly stateDatabase: string;
  readonly knownFindings?: string;
}
export function startInvestigator(
  config: Config,
  options: InvestigatorOptions,
  publish: InvestigatorDependencies["post"],
): () => void {
  const client = new LensClient(config);
  const { repository, clientId, key, dailyLimit, model, scope } = options;
  const store = new ClickHouseState(
    options.stateConnection,
    options.stateDatabase,
    `investigator/${digest(JSON.stringify(scope))}`,
    stateSchema,
    initialState,
  );
  const controller = new AbortController();
  let timer: NodeJS.Timeout | undefined;
  let initialized = false;
  const run = async () => {
    if (controller.signal.aborted) return;
    const started = Date.now();
    let stage = "state claim";
    try {
      if (!initialized) {
        await bootstrapKnownFindings(store, scope, options.knownFindings);
        initialized = true;
      }
      await cycle(
        {
          store,
          model,
          dailyLimit,
          sample: (signal) => {
            stage = "trace sample";
            return client.recent(signal, 100);
          },
          analyze: async (sample, previous, signal) => {
            stage = "repository access";
            const repo = new Repository(repository, clientId, key);
            await repo.initialize(signal);
            stage = "evidence and model analysis";
            const evidence = new Evidence(sample, client, config.agent);
            const candidate = await investigate(
              config,
              model,
              evidence,
              repo,
              previous,
              signal,
            );
            stage = "analysis persistence";
            return {
              candidate: candidate?.frequency ? candidate : undefined,
              sha: repo.sha,
            };
          },
          post: (candidate, sample, source) => {
            stage = "report delivery";
            return publish(candidate, sample, source);
          },
          persist: (candidate, sample, source, signal) => {
            stage = "native finding persistence";
            return client.persistFinding(candidate, sample, source, signal);
          },
        },
        AbortSignal.any([controller.signal, AbortSignal.timeout(240_000)]),
      );
    } catch {
      console.warn(
        `Lens investigator cycle incomplete at ${stage}; report delivery may be partial`,
      );
    }
    if (!controller.signal.aborted)
      timer = setTimeout(
        () => {
          void run();
        },
        Math.max(1000, started + 600_000 - Date.now()),
      );
  };
  console.info("Lens periodic investigator enabled");
  void run();
  return () => {
    controller.abort();
    clearTimeout(timer);
  };
}
