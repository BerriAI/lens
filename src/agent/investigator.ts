import { randomUUID } from "node:crypto";
import { z } from "zod";
import OpenAI from "openai";
import type { AgentConfig as Config } from "./config.js";
import { LensClient, LensReadError } from "./lens.js";
import { ClickHouseState, digest, type StateStore } from "./state.js";
import { Evidence, type Sample } from "./evidence.js";
import { Repository } from "./repository.js";
import { investigate } from "./agent.js";
import { leaseDuration, renewLease } from "./lease.js";
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
  last_check_at: z.number().default(0),
  last_outcome: z
    .enum([
      "starting",
      "running",
      "no_traces",
      "unchanged",
      "quiet",
      "duplicate",
      "delivered",
      "failed",
      "delivery_unknown",
      "run_limit",
    ])
    .default("starting"),
  last_error: z.string().default(""),
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
  last_check_at: 0,
  last_outcome: "starting",
  last_error: "",
  next_issue_number: 1,
  signature: "",
  sent: [],
  last_run: null,
};

function failureDetail(error: unknown, signal: AbortSignal): string {
  if (error instanceof LensReadError) return error.message;
  if (signal.aborted) return "Investigation canceled or ownership lost";
  if (error instanceof OpenAI.APIConnectionTimeoutError)
    return "Model gateway connection or request timed out";
  if (error instanceof OpenAI.APIConnectionError)
    return "Model gateway connection failed";
  if (error instanceof OpenAI.APIError && error.status)
    return `Model request failed (HTTP ${error.status})`;
  if (error instanceof z.ZodError)
    return "Investigation data validation failed";
  return "Analysis could not finish";
}

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
  ) => Promise<{ candidates: readonly VerifiedCandidate[]; sha: string }>;
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
  readonly intervalMinutes?: number;
  readonly maxFindings?: number;
  readonly now?: () => number;
}

export async function cycle(
  deps: InvestigatorDependencies,
  cancellation: AbortSignal,
): Promise<void> {
  const now = deps.now ?? Date.now;
  const current = await deps.store.read();
  const started = now();
  const intervalMs = (deps.intervalMinutes ?? 10) * 60_000;
  if (current.value.lease_until > started) return;
  const abandoned =
    current.value.last_outcome === "running" && current.value.lease_until > 0;
  if (current.value.next_at > started && !abandoned) return;
  const day = new Date(started).toISOString().slice(0, 10);
  const previous =
    current.value.day === day
      ? current.value
      : { ...current.value, day, runs: 0, posts: 0 };
  const claimed = await deps.store.commit(current, {
    ...previous,
    next_at: started + intervalMs,
    lease_until: started + leaseDuration,
    owner: randomUUID(),
    last_check_at: started,
    last_outcome: "running",
    last_error: "",
  });
  console.info("Lens investigator durable lease acquired");
  if (claimed.value.runs >= deps.dailyLimit) {
    await deps.store.commit(claimed, {
      ...claimed.value,
      next_at: Date.parse(`${day}T00:00:00Z`) + 86_400_000,
      lease_until: 0,
      last_outcome: "run_limit",
    });
    console.info(
      "Lens investigator daily analysis budget reached; resumes at UTC midnight",
    );
    return;
  }
  const lease = renewLease(deps.store, claimed.value.owner, cancellation, now);
  const { store, signal } = lease;
  try {
    const sample = await deps.sample(signal);
    if (!sample.traces.length) {
      await store.commit(claimed, {
        ...claimed.value,
        lease_until: 0,
        last_outcome: "no_traces",
      });
      return;
    }
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
    if (signature === claimed.value.signature) {
      await store.commit(claimed, {
        ...claimed.value,
        lease_until: 0,
        last_outcome: "unchanged",
      });
      return;
    }
    signal.throwIfAborted();
    const reserved = await store.commit(claimed, {
      ...claimed.value,
      runs: claimed.value.runs + 1,
    });
    const result = await deps.analyze(sample, reserved.value.sent, signal);
    signal.throwIfAborted();
    const source = {
      model: deps.model,
      prompt_revision: PROMPT_REVISION,
      trace_ids: sample.traces.map((row) => row.trace_id),
      repository_sha: result.sha,
      at: started,
    };
    const candidates = [...result.candidates]
      .sort(
        (a, b) =>
          b.candidate.impact - a.candidate.impact ||
          b.candidate.confidence - a.candidate.confidence,
      )
      .slice(0, deps.maxFindings ?? 1);
    if (!candidates.length) {
      await store.commit(reserved, {
        ...reserved.value,
        next_at: Math.max(reserved.value.next_at, now() + intervalMs),
        last_run: source,
        signature,
        lease_until: 0,
        last_outcome: "quiet",
      });
      console.info("Lens investigator analysis persisted; no new report");
      return;
    }
    for (const candidate of candidates) {
      const pending = await store.read();
      if (pending.value.owner !== reserved.value.owner || signal.aborted)
        return;
      const duplicate = pending.value.sent.find(
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
      const owned = await store.read();
      if (signal.aborted || owned.value.owner !== reserved.value.owner) return;
      if (duplicate) {
        await store.commit(owned, {
          ...owned.value,
          last_run: source,
          last_outcome: "duplicate",
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
        continue;
      }
      const committed = await store.commit(owned, {
        ...owned.value,
        next_at: Math.max(owned.value.next_at, now() + intervalMs),
        posts: owned.value.posts + 1,
        next_issue_number: owned.value.next_issue_number + 1,
        last_run: source,
        last_outcome: "delivery_unknown",
        sent: [
          ...owned.value.sent,
          {
            fingerprint: candidate.fingerprint,
            evidence_hash: candidate.evidenceHash,
            issue_key: candidate.candidate.issue_key,
            title: candidate.candidate.title,
            issue_number: owned.value.next_issue_number,
            native_finding: native,
            provenance: source,
          },
        ].slice(-200),
      });
      signal.throwIfAborted();
      await deps.post(
        { ...candidate, native, issueNumber: owned.value.next_issue_number },
        sample,
        source,
      );
      await store.commit(committed, {
        ...committed.value,
        last_outcome: "delivered",
      });
      console.info(
        "Lens investigator report delivered with durable provenance",
      );
    }
    const completed = await store.read();
    if (completed.value.owner !== reserved.value.owner) return;
    await store.commit(completed, {
      ...completed.value,
      signature,
      next_at: Math.max(completed.value.next_at, now() + intervalMs),
      lease_until: 0,
    });
  } catch (error) {
    await lease.stop();
    const failed = await deps.store.read();
    if (failed.value.owner === claimed.value.owner) {
      await deps.store.commit(failed, {
        ...failed.value,
        next_at:
          failed.value.next_issue_number === claimed.value.next_issue_number
            ? Math.min(failed.value.next_at, now() + 600_000)
            : failed.value.next_at,
        lease_until: 0,
        last_outcome:
          failed.value.last_outcome === "delivery_unknown"
            ? "delivery_unknown"
            : "failed",
        last_error:
          failed.value.last_outcome === "delivery_unknown"
            ? "Slack delivery could not be confirmed. The finding is saved in Lens; the message will not be blindly resent."
            : `${failureDetail(error, signal)}. The same evidence remains eligible for the next check.`,
      });
    }
    throw error;
  } finally {
    await lease.stop();
  }
}

export interface InvestigatorOptions {
  readonly repository: string;
  readonly repositoryAccess?: "public";
  readonly clientId: string;
  readonly key: string;
  readonly dailyLimit: number;
  readonly intervalMinutes?: number;
  readonly maxFindings?: number;
  readonly model: string;
  readonly scope: readonly string[];
  readonly stateConnection: string;
  readonly stateDatabase: string;
  readonly knownFindings?: string;
}
interface InvestigatorServices {
  readonly client: Pick<
    LensClient,
    "recent" | "get" | "link" | "persistFinding"
  >;
  readonly store: StateStore<State>;
  readonly repository: (signal: AbortSignal) => Promise<Repository>;
  readonly investigate: typeof investigate;
}
export function startInvestigator(
  config: Config,
  options: InvestigatorOptions,
  publish: InvestigatorDependencies["post"],
  services: Partial<InvestigatorServices> = {},
): () => void {
  const client = services.client ?? new LensClient(config);
  const { repository, clientId, key, dailyLimit, model, scope } = options;
  const store =
    services.store ??
    new ClickHouseState(
      options.stateConnection,
      options.stateDatabase,
      `investigator/${digest(JSON.stringify(scope))}`,
      stateSchema,
      initialState,
    );
  const repositoryFor =
    services.repository ??
    (async (signal: AbortSignal) => {
      const repo = new Repository(
        repository,
        clientId,
        key,
        fetch,
        options.repositoryAccess,
      );
      await repo.initialize(signal);
      return repo;
    });
  const controller = new AbortController();
  let timer: NodeJS.Timeout | undefined;
  let initialized = false;
  const run = async () => {
    if (controller.signal.aborted) return;
    const signal = controller.signal;
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
          intervalMinutes: options.intervalMinutes,
          maxFindings: options.maxFindings,
          sample: (signal) => {
            stage = "trace sample";
            return client.recent(signal, 100);
          },
          analyze: async (sample, previous, signal) => {
            stage = "repository access";
            const repo = await repositoryFor(signal);
            stage = "evidence and model analysis";
            const evidence = new Evidence(
              sample,
              client,
              config.agent,
              config.rootInput,
            );
            const candidates = await (services.investigate ?? investigate)(
              config,
              model,
              evidence,
              repo,
              previous,
              signal,
              undefined,
              options.maxFindings,
            );
            stage = "analysis persistence";
            return {
              candidates,
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
        signal,
      );
    } catch (error) {
      console.warn(
        "Lens investigator cycle incomplete " +
          JSON.stringify({
            agent: config.agent,
            stage,
            reason: failureDetail(error, signal),
          }),
      );
    }
    if (!controller.signal.aborted)
      timer = setTimeout(() => {
        void run();
      }, 15_000);
  };
  console.info("Lens periodic investigator enabled");
  void run();
  return () => {
    controller.abort();
    clearTimeout(timer);
  };
}
