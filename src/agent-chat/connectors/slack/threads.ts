import { z } from "zod";
import type { Config } from "./config.js";
import type { Turn } from "@litellm/lens-agent/models";
import type { NativeFinding } from "@litellm/lens-agent/models";
import type { VerifiedCandidate } from "@litellm/lens-agent/findings";
import type { Sample } from "@litellm/lens-agent/evidence";
import { traceSchema } from "@litellm/lens-agent/lens";
import {
  ClickHouseState,
  clickhouseConnection,
  digest,
  type StateStore,
} from "@litellm/lens-agent/state";

export const threadSchema = z.object({
  title: z.string().max(300),
  issue: z.string().max(100),
  traces: z
    .array(
      traceSchema.extend({
        url: z.string().url(),
        span_id: z.string().optional(),
      }),
    )
    .min(1)
    .max(4),
  turns: z
    .array(
      z.object({
        role: z.enum(["user", "assistant"]),
        content: z.string().max(3000),
      }),
    )
    .max(6),
  handled: z.array(z.string()).max(20),
});
export type FindingThread = z.infer<typeof threadSchema>;
export function candidateThread(
  verified: Pick<VerifiedCandidate, "issueNumber" | "evidenceLinks"> & {
    native?: NativeFinding;
    candidate: Pick<VerifiedCandidate["candidate"], "title" | "evidence">;
  },
  sample: Pick<Sample, "traces">,
): FindingThread {
  return threadSchema.parse({
    issue:
      verified.native?.findingId ??
      String(verified.issueNumber ?? "unassigned"),
    title: verified.candidate.title,
    traces: verified.candidate.evidence.map((item, index) => {
      const trace = sample.traces.find((row) => row.trace_id === item.trace_id);
      if (!trace) throw new Error("Finding trace is unavailable");
      return {
        ...trace,
        span_id: item.span_id,
        url: verified.evidenceLinks[index],
      };
    }),
    turns: [],
    handled: [],
  });
}
export interface ThreadRegistry {
  read(thread: string): Promise<FindingThread | null>;
  register(thread: string, finding: FindingThread): Promise<void>;
  claim(thread: string, message: string): Promise<FindingThread | null>;
  remember(thread: string, turns: readonly Turn[]): Promise<void>;
}

export class FindingThreads implements ThreadRegistry {
  constructor(
    private readonly store: (
      thread: string,
    ) => StateStore<FindingThread | null>,
  ) {}

  async read(thread: string): Promise<FindingThread | null> {
    return (await this.store(thread).read()).value;
  }

  async register(thread: string, finding: FindingThread): Promise<void> {
    const store = this.store(thread);
    const previous = await store.read();
    if (previous.value) return;
    await store.commit(previous, threadSchema.parse(finding));
  }

  async claim(thread: string, message: string): Promise<FindingThread | null> {
    const store = this.store(thread);
    const previous = await store.read();
    if (!previous.value || previous.value.handled.includes(message))
      return null;
    const next = {
      ...previous.value,
      handled: [...previous.value.handled, message].slice(-20),
    };
    await store.commit(previous, next);
    return next;
  }

  async remember(thread: string, turns: readonly Turn[]): Promise<void> {
    const store = this.store(thread);
    const previous = await store.read();
    if (!previous.value) return;
    await store.commit(previous, {
      ...previous.value,
      turns: turns
        .slice(-6)
        .map((turn) => ({ ...turn, content: turn.content.slice(0, 3000) })),
    });
  }
}

export function findingThreads(
  config: Config,
  env: NodeJS.ProcessEnv,
): FindingThreads {
  const scope = digest(
    JSON.stringify([config.workspace, config.channel, config.agent]),
  );
  return new FindingThreads((thread) => {
    if (!/^\d+\.\d+$/.test(thread)) throw new Error("Invalid finding thread");
    return new ClickHouseState(
      clickhouseConnection(env),
      env.CLICKHOUSE_DATABASE || "lens",
      `slack-thread/${scope}/${thread}`,
      threadSchema.nullable(),
      null,
    );
  });
}

export async function bootstrapThreads(
  registry: ThreadRegistry,
  config: Config,
  raw?: string,
): Promise<void> {
  if (!raw) return;
  if (raw.length > 64_000) throw new Error("Finding bootstrap exceeds limit");
  const seeds = z
    .array(
      z.object({
        thread: z.string().regex(/^\d+\.\d+$/),
        finding: threadSchema,
      }),
    )
    .max(10)
    .parse(JSON.parse(raw));
  for (const seed of seeds) {
    if (
      seed.finding.traces.some((trace) => {
        const url = new URL(trace.url);
        return (
          (!trace.agent_names.includes(config.agent) &&
            trace.service !== (config.service ?? config.agent)) ||
          url.origin !== new URL(config.publicUrl).origin ||
          url.searchParams.get("trace") !== trace.trace_id ||
          url.searchParams.get("trace_ref") !== trace.trace_ref
        );
      })
    )
      throw new Error("Finding bootstrap scope does not match");
    await registry.register(seed.thread, seed.finding);
  }
}
