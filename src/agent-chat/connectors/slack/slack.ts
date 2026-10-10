import { redact } from "@litellm/lens-agent/evidence";

import type { Answer, Category, Frequency } from "@litellm/lens-agent/models";
export type {
  Answer,
  Category,
  Frequency,
  Opportunity,
  MeasuredFrequency,
} from "@litellm/lens-agent/models";
export const categories: readonly Category[] = [
  "Performance",
  "Agent quality",
  "Reliability",
];

export const detailed = (text: string): boolean =>
  /\b(details?|detailed|explain|why|evidence|breakdown|deeper)\b/i.test(text);
export const escapeSlack = (text: string): string =>
  text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
export function words(text: string, limit: number): string {
  const matches = [...text.matchAll(/\S+/g)];
  return matches.length > limit
    ? `${text.slice(0, matches[limit]!.index).trimEnd()}…`
    : text;
}
export function prose(text: string): string {
  return redact(text)
    .replace(/\[([^\]]+)\]\(https?:\/\/[^\s)]+\)/g, "$1")
    .replace(/https?:\/\/[^\s<>]+/g, "")
    .replace(/```(?:[\w+-]+)?\n?/g, "")
    .replace(/^#{1,6}\s+/gm, "")
    .replace(/[ \t]+\n/g, "\n")
    .trim();
}
export function safeAnswer(
  answer: Answer,
  allowed: ReadonlySet<string>,
  detail: boolean,
  frequencies: Readonly<Record<string, Frequency>> = {},
  sourceLimit: 2 | 3 | 4 = 2,
): Answer {
  return {
    title: prose(answer.title).slice(0, 100),
    summary: words(prose(answer.summary), detail ? 160 : 80),
    sources: answer.sources
      .filter((source) => allowed.has(source.url))
      .filter(
        (source, index, all) =>
          all.findIndex((item) => item.url === source.url) === index,
      )
      .slice(0, sourceLimit)
      .map((source) => ({
        label: prose(source.label).slice(0, 60) || "Open evidence",
        url: source.url,
      })),
    ...(answer.opportunities
      ? {
          opportunities: answer.opportunities
            .slice(0, 3)
            .filter(
              (item) =>
                Number.isInteger(item.impact) &&
                item.impact >= 1 &&
                item.impact <= 10,
            )
            .map((item) => {
              const selected = frequencies[item.frequency_metric];
              const sources = item.sources
                .filter((source) => allowed.has(source.url))
                .slice(0, 2);
              const metric =
                selected?.category === item.category &&
                selected.title === item.metric_title &&
                sources.length &&
                sources.every((source) =>
                  selected.affected_trace_ids?.includes(
                    new URL(source.url).searchParams.get("trace") ?? "",
                  ),
                )
                  ? selected
                  : undefined;
              return {
                ...item,
                summary: words(prose(item.summary), detail ? 45 : 25),
                frequency_metric: metric?.label ?? "unknown",
                frequency:
                  typeof metric?.count === "number" &&
                  typeof metric.total === "number" &&
                  metric.total > 0 &&
                  metric.title &&
                  metric.unit
                    ? {
                        count: metric.count,
                        total: metric.total,
                        label: metric.label,
                        title: metric.title,
                        unit: metric.unit,
                      }
                    : undefined,
                sources,
              };
            }),
        }
      : {}),
  };
}
export function answerBlocks(answer: Answer) {
  if (answer.opportunities?.length) {
    let linkIndex = 0;
    return categories.map((category) => {
      const opportunities = answer.opportunities!.filter(
        (item) => item.category === category,
      );
      const text = opportunities.length
        ? opportunities
            .map((item) => {
              const links = item.sources
                .map(
                  (source) =>
                    `<${source.url.replace(/[<>|]/g, encodeURIComponent)}|${new URL(source.url).searchParams.has("trace") ? "Trace" : "Evidence"} ${++linkIndex}>`,
                )
                .join(" · ");
              return `${item.frequency ? `*${escapeSlack(item.frequency.title)}*\n` : ""}${escapeSlack(item.summary).replace(/\*\*([^*]+)\*\*/g, "*$1*")}\nFrequency: ${escapeSlack(item.frequency_metric)} · Impact ${item.impact}/10 (assessment)${links ? `\n${links}` : ""}`;
            })
            .join("\n\n")
        : "No supported finding";
      return {
        type: "section" as const,
        text: {
          type: "mrkdwn" as const,
          text: `*${category}*\n${text}`,
          verbatim: true,
        },
      };
    });
  }
  const summary = escapeSlack(answer.summary).replace(
    /\*\*([^*]+)\*\*/g,
    "*$1*",
  );
  return [
    {
      type: "header" as const,
      text: {
        type: "plain_text" as const,
        text: answer.title || "Lens",
        emoji: false,
      },
    },
    {
      type: "section" as const,
      text: {
        type: "mrkdwn" as const,
        text: summary || "No supported answer is available",
        verbatim: true,
      },
    },
    ...(answer.sources.length
      ? [
          {
            type: "context" as const,
            elements: [
              {
                type: "mrkdwn" as const,
                text: answer.sources
                  .map(
                    (source) =>
                      `<${source.url.replace(/[<>|]/g, encodeURIComponent)}|${escapeSlack(source.label).replace(/\|/g, "&#124;")}>`,
                  )
                  .join("  ·  "),
                verbatim: true,
              },
            ],
          },
        ]
      : []),
  ];
}
