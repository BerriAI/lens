import { App, LogLevel } from "@slack/bolt";
import { WebClient } from "@slack/web-api";
import { replyAgent } from "./reply.js";
import { Chat } from "./chat.js";
import { configFrom, agentConfig, investigatorOptions } from "./config.js";
import { startInvestigator } from "@litellm/lens-agent/investigator";
import { postCandidate } from "./findings.js";
import { slackReplies } from "./responses.js";
import { within } from "./completion.js";
import { bootstrapThreads, findingThreads } from "./threads.js";
import { workingStatus, reactionStatus } from "./activity.js";

async function main() {
  const config = configFrom(process.env);
  if (!config) return;
  const logger = {
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
  };
  const app = new App({
    token: config.botToken,
    appToken: config.appToken,
    socketMode: true,
    logLevel: LogLevel.ERROR,
    logger,
    clientOptions: {
      retryConfig: { retries: 0 },
      rejectRateLimitedCalls: true,
      timeout: 10_000,
    },
  });
  const identity = await app.client.auth.test();
  if (identity.team_id !== config.workspace || !identity.user_id)
    throw new Error("Slack token workspace does not match its allowlist");
  const registry = findingThreads(config, process.env);
  await bootstrapThreads(
    registry,
    config,
    process.env.LENS_SLACK_FINDING_THREADS,
  );
  const activityClient = new WebClient(config.botToken, {
    logger,
    timeout: 2000,
    retryConfig: { retries: 0 },
    rejectRateLimitedCalls: true,
    logLevel: LogLevel.ERROR,
  });
  const status = workingStatus((args) =>
    activityClient.assistant.threads.setStatus(args),
  );
  const reactions = reactionStatus(activityClient);
  const responses = slackReplies(app.client);
  const chat = new Chat(
    config,
    replyAgent(config),
    responses.reply,
    Date.now,
    registry,
    async (event, state) => {
      await Promise.allSettled([
        status(event, state),
        ...(process.env.LENS_SLACK_REACTIONS_ENABLED === "true"
          ? [reactions(event, state)]
          : []),
      ]);
    },
    { acknowledge: responses.acknowledge },
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
      bot: "bot_id" in event || event.user === identity.user_id,
    });
  });
  if (process.env.LENS_SLACK_THREAD_REPLIES_ENABLED === "true") {
    app.event("message", async ({ event, body }) => {
      if (event.subtype && event.subtype !== "file_share") return;
      if (!("user" in event) || !("text" in event) || !event.text) return;
      await chat.mention({
        id: body.event_id,
        workspace: body.team_id,
        channel: event.channel,
        user: event.user ?? "",
        text: event.text,
        ts: event.ts,
        thread: event.thread_ts,
        bot: "bot_id" in event || event.user === identity.user_id,
        addressed: event.text.includes(`<@${identity.user_id}>`),
      });
    });
  }
  app.error(async () => {
    console.error("Slack chat request failed");
  });
  let stopping = false;
  for (const signal of ["SIGINT", "SIGTERM"] as const)
    process.once(signal, () => {
      if (stopping) return;
      stopping = true;
      stopInvestigator();
      void Promise.allSettled([
        chat.shutdown(8000),
        within(app.stop(), 8000),
      ]).finally(() => process.exit(0));
    });
  await app.start();
  console.info("Slack mention agent connected");
  try {
    const options = investigatorOptions(config, process.env);
    if (options)
      stopInvestigator = startInvestigator(
        agentConfig(config),
        options,
        (candidate, sample, source) =>
          postCandidate(
            app.client,
            config,
            candidate,
            sample,
            source,
            registry,
          ),
      );
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
