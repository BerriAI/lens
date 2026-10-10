import { randomUUID } from "node:crypto";
import type { WebClient } from "@slack/web-api";
import { z } from "zod";
import type { Config } from "./config.js";
import { LensClient } from "./lens.js";
import {
  ClickHouseState,
  clickhouseConnection,
  digest,
  type StateStore,
} from "./investigator-state.js";
import { Evidence, type Sample } from "./investigator-evidence.js";
import { Repository } from "./investigator-repo.js";
import {
  investigate,
  PROMPT_REVISION,
  type VerifiedCandidate,
} from "./investigator-agent.js";
import { escapeSlack, prose, words } from "./slack.js";
import {
  prepareChart,
  renderChartPng,
  type ChartPayload,
  type ChartSlack,
} from "./chart.js";
import { updateWithChart } from "./slack-transport.js";

const provenance = z.object({
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
    candidate: VerifiedCandidate,
    sample: Sample,
    source: z.infer<typeof provenance>,
  ) => Promise<void>;
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
  const duplicate =
    candidate &&
    reserved.value.sent.some(
      (item) =>
        item.fingerprint === candidate.fingerprint ||
        item.evidence_hash === candidate.evidenceHash ||
        item.title === candidate.candidate.title,
    );
  if (!candidate || duplicate) {
    await deps.store.commit(reserved, {
      ...reserved.value,
      last_run: source,
      signature,
      lease_until: 0,
    });
    console.info("Lens investigator analysis persisted; no new report");
    return;
  }
  const committed = await deps.store.commit(reserved, {
    ...reserved.value,
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
        provenance: source,
      },
    ].slice(-200),
  });
  if (now() >= committed.value.lease_until - 15_000 || signal.aborted) return;
  await deps.post(
    { ...candidate, issueNumber: reserved.value.next_issue_number },
    sample,
    source,
  );
  console.info("Lens investigator report delivered with durable provenance");
}

function markdown(text: string) {
  return {
    type: "section" as const,
    text: {
      type: "mrkdwn" as const,
      text: text.slice(0, 2900),
      verbatim: true,
    },
  };
}

export async function postCandidate(
  slack: Pick<WebClient, "chat"> & ChartSlack,
  config: Config,
  verified: VerifiedCandidate,
  sample: Sample,
  source: z.infer<typeof provenance>,
): Promise<void> {
  const candidate = verified.candidate;
  const icon = {
    issue: "🔎",
    regression: "📉",
    opportunity: "💡",
    frustration: "😤",
    quiet: "",
  }[candidate.kind];
  const category =
    candidate.category === "Agent quality" ? "Quality" : candidate.category;
  const title =
    `${candidate.kind === "opportunity" ? "Feature" : "Bug Fix"}: ${category} — ${prose(candidate.title)}`.slice(
      0,
      150,
    );
  const users = [
    ...new Set(
      verified.receipts?.flatMap((receipt) =>
        receipt.user ? [receipt.user] : [],
      ) ?? [],
    ),
  ].slice(0, 3);
  const frequency = verified.frequency
    ? `${Math.round((100 * verified.frequency.count) / verified.frequency.total)}% (${verified.frequency.count}/${verified.frequency.total}) — ${verified.frequency.label.replace(/^\d+\/\d+ /, "")}`
    : "unknown";
  const links = verified.evidenceLinks
    .slice(0, 3)
    .map((url, index) => `<${url}|Trace ${index + 1}>`)
    .join(" · ");
  if (!verified.frequency)
    throw new Error("A report needs a verified frequency cohort");
  const chartPayload: ChartPayload = {
    title: verified.frequency.support
      ? "Observed request support"
      : verified.frequency.title.slice(0, 80),
    category,
    affected: verified.frequency.count,
    total: verified.frequency.total,
    unit: verified.frequency.unit,
    ...(verified.frequency.support
      ? {
          affectedLabel: "Supporting requests",
          otherLabel: "Other reviewed requests",
        }
      : {}),
  };
  const png = await renderChartPng(chartPayload);
  const parent = {
    channel: config.channel,
    text: "Lens found an improvement candidate",
    attachments: [
      {
        color: "#8058F4",
        blocks: [
          {
            type: "header" as const,
            text: { type: "plain_text" as const, text: title, emoji: true },
          },
          markdown(
            `*Lens Issue #${verified.issueNumber ?? "unassigned"} · Proposed*\n*Impact Score:* ${candidate.impact}/10 (assessment)\n*Helps Users:* ${users.length ? users.map(escapeSlack).join(", ") : "user identity unavailable"}${links ? ` · ${links}` : ""}\n*${verified.frequency.support ? "Observed request support" : "Frequency"}:* ${escapeSlack(frequency)}\n*What this can improve:* If validated, ${escapeSlack(words(prose(candidate.outcome), 25))}`,
          ),
          {
            type: "context" as const,
            elements: [
              {
                type: "mrkdwn" as const,
                text: `${icon} ${category} · Proposed change · Confidence ${Math.round(candidate.confidence * 100)}/100 (uncalibrated assessment)`,
                verbatim: true,
              },
            ],
          },
        ],
      },
    ],
    parse: "none" as const,
    unfurl_links: false,
    unfurl_media: false,
  };
  const posted = await slack.chat.postMessage(parent);
  if (!posted.ts) throw new Error("Slack did not return a thread");
  const cited = sample.traces.filter((row) =>
    candidate.evidence.some((item) => item.trace_id === row.trace_id),
  );
  const metrics = [
    "Observed trace sample (not a benchmark)",
    "Trace | Status | Seconds | LLM calls | Tool calls",
    ...cited.map(
      (row) =>
        `${row.trace_id.slice(0, 10)} | ${row.status} | ${(row.duration_ms / 1000).toFixed(1)} | ${row.llm_calls} | ${row.tool_calls}`,
    ),
  ].join("\n");
  const max = Math.max(...cited.map((row) => row.duration_ms), 1);
  const graph = [
    "Observed duration; different tasks may not be comparable",
    ...cited.map(
      (row) =>
        `${row.trace_id.slice(0, 8)} ${"█".repeat(Math.max(1, Math.round((row.duration_ms / max) * 16)))} ${(row.duration_ms / 1000).toFixed(1)}s`,
    ),
  ].join("\n");
  const excerpts = candidate.evidence
    .map(
      (item, index) =>
        `>${escapeSlack(item.quote).replace(/\n/g, "\n>")}\n<${verified.evidenceLinks[index]}|Open trace ${index + 1}>`,
    )
    .join("\n\n");
  const receipts = verified.receipts
    ?.slice(0, 3)
    .map(
      (receipt, index) =>
        `*Trace ${index + 1}*${receipt.user ? ` · ${escapeSlack(receipt.user)}` : ""}\n*User tried*\n>${escapeSlack(words(prose(receipt.request), 35) || "User request unavailable").replace(/\n/g, "\n>")}\n*Agent behavior*\n>${escapeSlack(words(prose(receipt.behavior), 35) || "Final response unavailable").replace(/\n/g, "\n>")}\n*Observed failure or gap*\n>${escapeSlack(receipt.quote).replace(/\n/g, "\n>")}\n<${receipt.url}|Trace ${index + 1}>`,
    )
    .join("\n\n");
  const details = [
    markdown(receipts || excerpts),
    markdown(
      `*Observed issue*\n${escapeSlack(words(prose(candidate.observation), 80))}\n*Frequency:* ${escapeSlack(frequency)}`,
    ),
    markdown(
      `*Code hypothesis*\n${escapeSlack(words(prose(candidate.code_hypothesis), 80))}\n${verified.codeLinks.map((url, index) => `<${url}|Code reference ${index + 1}>`).join(" · ")}`,
    ),
    markdown(`\`\`\`\n${escapeSlack(metrics)}\n\`\`\``),
    markdown(`\`\`\`\n${escapeSlack(graph)}\n\`\`\``),
    markdown(
      `*Required experiment*\n${escapeSlack(words(prose(candidate.experiment), 100))}\n\n*Limitations*\n${escapeSlack(words(prose(candidate.limitation), 60))}\nBounded sample; distinct trace IDs may share an incident. Current code is not verified deployed code. No measured improvement yet`,
    ),
    {
      type: "context" as const,
      elements: [
        {
          type: "plain_text" as const,
          text: `Model ${source.model} · ${source.prompt_revision} · Repository ${source.repository_sha.slice(0, 12)}`,
          emoji: false,
        },
      ],
    },
  ];
  const chart = await prepareChart(
    slack,
    chartPayload,
    { channel: config.channel, thread: posted.ts, blocks: details },
    png,
  );
  await updateWithChart(slack, {
    channel: config.channel,
    ts: posted.ts,
    text: parent.text,
    parse: "none",
    attachments: parent.attachments.map((attachment) => ({
      ...attachment,
      blocks: [
        ...attachment.blocks.slice(0, 2),
        chart.block,
        ...attachment.blocks.slice(2),
      ],
    })),
  });
}

export function startInvestigator(
  config: Config,
  slack: Pick<WebClient, "chat"> & ChartSlack,
  env: NodeJS.ProcessEnv = process.env,
): () => void {
  if (env.LENS_INVESTIGATOR_ENABLED !== "true") return () => {};
  const repository = env.LENS_INVESTIGATOR_REPOSITORY;
  const clientId = env.LENS_GITHUB_CLIENT_ID;
  const key = env.LENS_GITHUB_PRIVATE_KEY;
  const dailyLimit = Number(env.LENS_INVESTIGATOR_DAILY_RUN_LIMIT ?? "144");
  if (
    !repository ||
    !clientId ||
    !key ||
    !Number.isInteger(dailyLimit) ||
    dailyLimit < 1 ||
    dailyLimit > 144
  ) {
    console.error(
      "Investigator disabled: repository, GitHub App, ClickHouse URL or daily limit is missing or invalid",
    );
    return () => {};
  }
  const model = env.LENS_INVESTIGATOR_MODEL?.trim() || "gpt-6-astra";
  const client = new LensClient(config);
  const scope = [config.workspace, config.channel, config.agent, repository];
  const store = new ClickHouseState(
    clickhouseConnection(env),
    env.CLICKHOUSE_DATABASE || "lens",
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
        await bootstrapKnownFindings(
          store,
          scope,
          env.LENS_INVESTIGATOR_KNOWN_FINDINGS,
        );
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
            stage = "Slack report delivery";
            return postCandidate(slack, config, candidate, sample, source);
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
