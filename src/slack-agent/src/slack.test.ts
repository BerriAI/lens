import assert from "node:assert/strict";
import { test } from "node:test";
import { answerBlocks, safeAnswer, prose, words } from "./slack.js";
import { clickhouseConnection } from "./investigator-state.js";

test("compact Slack rendering preserves paragraphs, formats bold and blocks injected mentions", () => {
  const url = "https://lens.example.com/?trace=verified";
  const answer = safeAnswer(
    {
      title: "Two issues",
      summary:
        "**Observed**\nFirst paragraph\n\nSecond paragraph <!channel> <@U123> https://lens.example.com/?trace=raw",
      sources: [
        { label: "Open affected run", url },
        { label: "Invented", url: "https://lens.example.com/?trace=made-up" },
      ],
    },
    new Set([url]),
    false,
  );
  const blocks = answerBlocks(answer);
  const rendered = JSON.stringify(blocks);
  assert(
    rendered.includes("*Observed*\\nFirst paragraph\\n\\nSecond paragraph"),
  );
  assert(!rendered.includes("**Observed**"));
  assert(!rendered.includes("<!channel>") && !rendered.includes("<@U123>"));
  assert(rendered.includes(`|Open affected run>`));
  assert(!rendered.includes("made-up") && !rendered.includes("?trace=raw"));
  assert.equal(answer.sources.length, 1);
});

test("host enforces concise default answers and strips code fences without collapsing lines", () => {
  const input = `First paragraph\n\n${"word ".repeat(200)}`;
  assert.equal(words(input, 80).split(/\s+/).length, 80);
  assert(words(input, 80).includes("\n\n"));
  assert.equal(prose("```text\nline one\nline two\n```"), "line one\nline two");
  const answer = safeAnswer(
    { title: "t".repeat(120), summary: input, sources: [] },
    new Set(),
    true,
  );
  assert.equal(answer.title.length, 100);
  assert.equal(answer.summary.split(/\s+/).length, 160);
  assert(
    !prose(
      "Captured ghp_private123 rnd_private123 and SLACK_BOT_TOKEN=private",
    ).includes("private"),
  );
});

test("ClickHouse split settings retain credentials internally and match the server listener defaults", () => {
  const url = new URL(
    clickhouseConnection({
      CLICKHOUSE_HOST: "internal-db",
      CLICKHOUSE_USER: "reader",
      CLICKHOUSE_PASSWORD: "secret@value",
    }),
  );
  assert.equal(url.hostname, "internal-db");
  assert.equal(url.port, "8123");
  assert.equal(decodeURIComponent(url.password), "secret@value");
  assert.equal(
    clickhouseConnection({ CLICKHOUSE_URL: "https://db.example.com:8443" }),
    "https://db.example.com:8443",
  );
});

test("categorized answers use only host-computed matching frequencies and numbered verified evidence", () => {
  const url = "https://lens.example.com/?trace=actual";
  const answer = safeAnswer(
    {
      title: "Report",
      summary: "",
      sources: [],
      opportunities: [
        {
          category: "Performance",
          summary: "Slow startup",
          impact: 7,
          frequency_metric: "delay",
          metric_title: "Wait before model",
          sources: [{ label: "run", url }],
        },
        {
          category: "Agent quality",
          summary: "Needs feedback",
          impact: 6,
          frequency_metric: "delay",
          sources: [
            { label: "fake", url: "https://lens.example.com/?trace=invented" },
          ],
        },
      ],
    },
    new Set([url]),
    false,
    {
      delay: {
        category: "Performance",
        label: "6/6 complete one-model turns waited",
        title: "Wait before model",
        affected_trace_ids: ["actual"],
      },
    },
  );
  assert.equal(
    answer.opportunities?.[0]?.frequency_metric,
    "6/6 complete one-model turns waited",
  );
  assert.equal(answer.opportunities?.[1]?.frequency_metric, "unknown");
  const rendered = JSON.stringify(answerBlocks(answer));
  assert(rendered.includes("|Trace 1>"));
  assert(!rendered.includes("invented"));
  assert(rendered.includes("No supported finding"));
});
