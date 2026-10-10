import type { WebClient } from "@slack/web-api";
import { answerBlocks } from "./slack.js";
import type { Reply } from "./chat.js";
import { prepareChart, renderChartPng, type ChartSlack } from "./chart.js";
import { updateWithChart } from "./slack-transport.js";

export function slackReplies(slack: Pick<WebClient, "chat"> & ChartSlack) {
  const acknowledge = async (
    channel: string,
    thread: string,
  ): Promise<string> => {
    const text =
      "On it. I’m checking the evidence and will update this message with the result";
    const posted = await slack.chat.postMessage({
      channel,
      thread_ts: thread,
      text,
      parse: "none",
      unfurl_links: false,
      unfurl_media: false,
      blocks: [{ type: "section", text: { type: "plain_text", text } }],
    });
    if (!posted.ts)
      throw new Error("Slack did not return an acknowledgment timestamp");
    return posted.ts;
  };
  const reply: Reply = async (channel, thread, answer, message, signal) => {
    signal?.throwIfAborted();
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
    signal?.throwIfAborted();
    const blocks = answerBlocks(answer);
    const text = answer.title;
    const posted = message
      ? await slack.chat.update({
          channel,
          ts: message,
          text,
          parse: "none",
          blocks,
        })
      : await slack.chat.postMessage({
          channel,
          thread_ts: thread,
          text,
          unfurl_links: false,
          unfurl_media: false,
          parse: "none",
          blocks,
        });
    if (payload && png) {
      signal?.throwIfAborted();
      const ts = message ?? posted.ts;
      if (!ts) throw new Error("Slack did not return a report timestamp");
      const chart = await prepareChart(
        slack,
        payload,
        { channel, thread },
        png,
      );
      signal?.throwIfAborted();
      await updateWithChart(
        slack,
        {
          channel,
          ts,
          text,
          parse: "none",
          blocks: [...blocks, chart.block],
        },
        undefined,
        signal,
      );
    }
  };
  return { acknowledge, reply };
}
