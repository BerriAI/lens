import { App, LogLevel } from "@slack/bolt";
import { responder } from "./agent.js";
import { Chat } from "./chat.js";
import { configFrom } from "./config.js";
import { startInvestigator } from "./investigator.js";
import { answerBlocks } from "./slack.js";
import { prepareChart, renderChartPng } from "./chart.js";
import { updateWithChart } from "./slack-transport.js";

async function main() {
  const config = configFrom(process.env);
  if (!config) return;
  const app = new App({
    token: config.botToken,
    appToken: config.appToken,
    socketMode: true,
    logLevel: LogLevel.ERROR,
    logger: {
      debug() {},
      info() {},
      warn() {
        console.warn("Slack chat warning");
      },
      error() {
        console.error("Slack chat transport error");
      },
      setLevel() {},
      getLevel() {
        return LogLevel.ERROR;
      },
      setName() {},
    },
    clientOptions: {
      retryConfig: { retries: 0 },
      rejectRateLimitedCalls: true,
      timeout: 10_000,
    },
  });
  const chat = new Chat(
    config,
    responder(config),
    async (channel, thread, answer) => {
      const measured = answer.opportunities?.find((item) => item.frequency);
      if (answer.opportunities?.length && !measured?.frequency)
        throw new Error("A report needs verified chart data");
      const payload = measured?.frequency
        ? {
            title: measured.frequency.title.slice(0, 80),
            category: measured.category,
            affected: measured.frequency.count,
            total: measured.frequency.total,
            unit: measured.frequency.unit,
          }
        : undefined;
      const png = payload ? await renderChartPng(payload) : undefined;
      const blocks = answerBlocks(answer);
      const posted = await app.client.chat.postMessage({
        channel,
        thread_ts: thread,
        text: "Lens replied in this thread",
        unfurl_links: false,
        unfurl_media: false,
        parse: "none",
        blocks,
      });
      if (payload && png) {
        if (!posted.ts)
          throw new Error("Slack did not return a report timestamp");
        try {
          const chart = await prepareChart(
            app.client,
            payload,
            { channel, thread },
            png,
          );
          await updateWithChart(app.client, {
            channel,
            ts: posted.ts,
            text: "Lens replied in this thread",
            parse: "none",
            blocks: [...blocks, chart.block],
          });
        } catch (error) {
          console.error("Slack reply chart delivery is incomplete");
          throw error;
        }
      }
    },
  );
  let stopInvestigator = () => {};
  app.event("app_mention", async ({ event, body }) => {
    await chat.mention({
      id: body.event_id,
      workspace: body.team_id,
      channel: event.channel,
      user: event.user ?? "",
      text: event.text,
      ts: event.ts,
      thread: event.thread_ts,
      bot: "bot_id" in event,
    });
  });
  app.error(async () => {
    console.error("Slack chat request failed");
  });
  for (const signal of ["SIGINT", "SIGTERM"] as const)
    process.once(signal, () => {
      stopInvestigator();
      void app.stop().finally(() => process.exit(0));
    });
  const identity = await app.client.auth.test();
  if (identity.team_id !== config.workspace)
    throw new Error("Slack token workspace does not match its allowlist");
  await app.start();
  console.info("Slack mention agent connected");
  try {
    stopInvestigator = startInvestigator(config, app.client);
  } catch {
    console.error(
      "Lens investigator configuration is invalid; mention chat remains available",
    );
  }
}

main().catch(() => {
  console.error(
    "Slack mention agent could not start; check its configuration and credentials",
  );
  process.exit(1);
});
