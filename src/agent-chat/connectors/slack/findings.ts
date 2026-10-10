import type { WebClient } from "@slack/web-api";
import type { z } from "zod";
import type { Config } from "./config.js";
import type { Sample } from "@litellm/lens-agent/evidence";
import type { PersistedCandidate } from "@litellm/lens-agent/findings";
import { provenance } from "@litellm/lens-agent/investigator";
import { escapeSlack, prose, words } from "./slack.js";
import {
  prepareChart,
  renderChartPng,
  type ChartPayload,
  type ChartSlack,
} from "./chart.js";
import { updateWithChart } from "./slack-transport.js";
import { candidateThread, type ThreadRegistry } from "./threads.js";
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
  verified: PersistedCandidate,
  sample: Sample,
  source: z.infer<typeof provenance>,
  registry?: ThreadRegistry,
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
            `*<${verified.native.url}|Finding #${escapeSlack(verified.native.findingId.replace(/^agent-/, "").slice(0, 8))}> · Proposed*\n*Impact Score:* ${candidate.impact}/10 (assessment)\n*Helps Users:* ${users.length ? users.map(escapeSlack).join(", ") : "user identity unavailable"}${links ? ` · ${links}` : ""}\n*${verified.frequency.support ? "Observed request support" : "Frequency"}:* ${escapeSlack(frequency)}\n*What this can improve:* If validated, ${escapeSlack(words(prose(candidate.outcome), 25))}`,
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
  if (registry)
    await registry.register(posted.ts, candidateThread(verified, sample));
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
