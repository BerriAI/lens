import type {
  AssistantThreadsSetStatusArguments,
  WebClient,
} from "@slack/web-api";
import type { Activity } from "./chat.js";

export function workingStatus(
  setStatus: (args: AssistantThreadsSetStatusArguments) => Promise<unknown>,
): Activity {
  const active = new Map<
    string,
    { timer: NodeJS.Timeout; update: Promise<void> }
  >();
  return async (event, state) => {
    const key = `${event.workspace}:${event.channel}:${event.ts}`;
    const send = async (status: string) => {
      try {
        await setStatus({
          channel_id: event.channel,
          thread_ts:
            event.thread && /^\d+\.\d+$/.test(event.thread)
              ? event.thread
              : event.ts,
          status,
        });
      } catch {
        console.warn("Slack working status is unavailable");
      }
    };
    if (state === "working") {
      if (active.has(key)) return;
      const current = {
        update: send("is working…"),
        timer: setInterval(() => {
          current.update = current.update.then(() => send("is working…"));
        }, 60_000),
      };
      current.timer.unref();
      active.set(key, current);
      await current.update;
      return;
    }
    const current = active.get(key);
    if (!current) return;
    active.delete(key);
    clearInterval(current.timer);
    await current.update;
    await send("");
  };
}

export function reactionStatus(slack: Pick<WebClient, "reactions">): Activity {
  const active = new Map<string, Promise<unknown>>();
  return async (event, state) => {
    const key = `${event.workspace}:${event.channel}:${event.ts}`;
    const target = { channel: event.channel, timestamp: event.ts };
    if (state === "working") {
      if (active.has(key)) return;
      const pending = slack.reactions
        .add({ ...target, name: "eyes" })
        .catch(() => {});
      active.set(key, pending);
      await pending;
      return;
    }
    const pending = active.get(key);
    if (!pending) return;
    active.delete(key);
    await pending;
    await Promise.allSettled([
      slack.reactions.add({
        ...target,
        name: state === "done" ? "white_check_mark" : "x",
      }),
      slack.reactions.remove({ ...target, name: "eyes" }),
    ]);
  };
}
