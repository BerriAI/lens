import { execFileSync } from "node:child_process";
import { mkdtemp, writeFile, rm } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import ts from "typescript";
import { Chat as Candidate } from "../dist/connectors/slack/chat.js";

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repository = resolve(packageRoot, "../..");
const base = process.argv[2] || "55d6a23";
const sourcePath = "src/agent-chat/connectors/slack/chat.ts";
const git = (...args) =>
  execFileSync("git", args, { cwd: repository, encoding: "utf8" }).trim();
const source = git("show", `${base}:${sourcePath}`);
const directory = await mkdtemp(join(packageRoot, ".ack-replay-"));
const fixtures = {
  status_delay_ms: 120,
  evidence_delay_ms: 180,
  slack_write_delay_ms: 10,
  trials: 5,
};
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const median = (values) =>
  [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)];
const round = (value) => Math.round(value * 10) / 10;
const quiet = console.info;
try {
  const path = join(directory, "baseline.mjs");
  await writeFile(
    path,
    ts.transpileModule(source, {
      compilerOptions: {
        target: ts.ScriptTarget.ES2023,
        module: ts.ModuleKind.ESNext,
      },
    }).outputText,
  );
  const { Chat: Baseline } = await import(pathToFileURL(path).href);
  console.info = () => {};
  const rows = [];
  for (let trial = 1; trial <= fixtures.trials; trial++) {
    for (const [version, Implementation] of [
      ["baseline", Baseline],
      ["candidate", Candidate],
    ]) {
      const started = performance.now();
      let firstText;
      let writes = 0;
      const visible = async () => {
        await pause(fixtures.slack_write_delay_ms);
        firstText ??= performance.now() - started;
        writes++;
      };
      const event = {
        id: "fixture",
        workspace: "TTEST",
        channel: "CTEST",
        user: "UUSER",
        text: "<@ULENS> inspect traces",
        ts: "1800000000.000000",
      };
      const options = {
        acknowledge: async () => {
          await visible();
          return "1800000000.123456";
        },
      };
      const chat = new Implementation(
        event,
        async () => {
          await pause(fixtures.evidence_delay_ms);
          return {
            title: "Fixture evidence",
            summary: "Deterministic fixture result",
            sources: [],
          };
        },
        visible,
        () => 1_800_000_000_000,
        undefined,
        async (_event, state) => {
          if (state === "working") await pause(fixtures.status_delay_ms);
        },
        options,
      );
      await chat.mention(event);
      rows.push({
        version,
        trial,
        first_text_ms: round(firstText),
        completion_ms: round(performance.now() - started),
        slack_writes: writes,
      });
    }
  }
  const metrics = Object.fromEntries(
    ["baseline", "candidate"].map((version) => {
      const selected = rows.filter((row) => row.version === version);
      return [
        version,
        {
          first_text_median_ms: round(
            median(selected.map((row) => row.first_text_ms)),
          ),
          completion_median_ms: round(
            median(selected.map((row) => row.completion_ms)),
          ),
        },
      ];
    }),
  );
  process.stdout.write(
    JSON.stringify(
      {
        scope:
          "Deterministic replay of actual Chat classes with delayed evidence, status and Slack fixtures; not production latency or model-quality evaluation.",
        baseline_ref: git("rev-parse", base),
        baseline_chat_blob: git("rev-parse", `${base}:${sourcePath}`),
        candidate_chat_blob: git("hash-object", sourcePath),
        node: process.version,
        fixtures,
        metrics,
        rows,
      },
      null,
      2,
    ) + "\n",
  );
} finally {
  console.info = quiet;
  await rm(directory, { recursive: true, force: true });
}
