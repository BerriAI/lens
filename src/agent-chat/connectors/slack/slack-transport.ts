import type {
  ChatPostMessageArguments,
  ChatUpdateArguments,
  WebClient,
} from "@slack/web-api";
import { z } from "zod";

const rejectedImage = z.object({
  code: z.literal("slack_webapi_platform_error"),
  data: z.object({
    ok: z.literal(false),
    error: z.enum(["invalid_blocks", "invalid_attachments"]),
    response_metadata: z.object({ messages: z.array(z.string()) }),
  }),
});

// Slack may acknowledge a private upload before its image block is ready.
// Only this definitive rejection is safe to retry; transport errors are ambiguous.
export async function postWithChart(
  slack: Pick<WebClient, "chat">,
  message: ChatPostMessageArguments,
  wait: (ms: number) => Promise<void> = (ms) =>
    new Promise((resolve) => setTimeout(resolve, ms)),
) {
  return retryChart(() => slack.chat.postMessage(message), wait);
}

export async function updateWithChart(
  slack: Pick<WebClient, "chat">,
  message: ChatUpdateArguments,
  wait: (ms: number) => Promise<void> = (ms) =>
    new Promise((resolve) => setTimeout(resolve, ms)),
) {
  return retryChart(() => slack.chat.update(message), wait);
}

async function retryChart<T>(
  operation: () => Promise<T>,
  wait: (ms: number) => Promise<void>,
): Promise<T> {
  const delays = [1000, 2000, 4000, 8000];
  for (let attempt = 0; ; attempt++) {
    try {
      return await operation();
    } catch (error) {
      const parsed = rejectedImage.safeParse(error);
      if (
        attempt >= delays.length ||
        !parsed.success ||
        !parsed.data.data.response_metadata.messages.some((text) =>
          /invalid slack file.*slack_file/i.test(text),
        )
      )
        throw error;
      await wait(delays[attempt]!);
    }
  }
}
