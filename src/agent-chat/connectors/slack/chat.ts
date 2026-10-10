import type { Config } from "./config.js";
import type { Turn } from "@litellm/lens-agent/models";
import type { ReplyAgent } from "./reply.js";
import type { Answer } from "./slack.js";
import type { FindingThread, ThreadRegistry } from "./threads.js";
import { redact } from "@litellm/lens-agent/evidence";
import {
  failureAnswer,
  RequestEnded,
  waitFor,
  within,
  withTimeout,
} from "./completion.js";

export interface Mention {
  readonly id: string;
  readonly workspace: string;
  readonly channel: string;
  readonly user: string;
  readonly text: string;
  readonly ts: string;
  readonly thread?: string;
  readonly bot?: boolean;
  readonly addressed?: boolean;
}
export type Reply = (
  channel: string,
  thread: string,
  answer: Answer,
  message?: string,
  signal?: AbortSignal,
) => Promise<void>;
export interface ChatOptions {
  readonly acknowledge?: (channel: string, thread: string) => Promise<string>;
  readonly timeoutMs?: number;
  readonly deliveryTimeoutMs?: number;
  readonly cleanupTimeoutMs?: number;
  readonly contextTimeoutMs?: number;
}
export type Activity = (
  event: Mention,
  state: "working" | "done" | "failed",
) => Promise<void>;

export class Chat {
  private readonly seen = new Map<string, number>();
  private readonly threads = new Map<
    string,
    { at: number; turns: readonly Turn[] }
  >();
  private requests: number[] = [];
  private busy = false;
  private stopping = false;
  private active?: { controller: AbortController; done: Promise<void> };

  constructor(
    private readonly config: Pick<Config, "workspace" | "channel">,
    private readonly respond: ReplyAgent,
    private readonly reply: Reply,
    private readonly now: () => number = Date.now,
    private readonly registry?: ThreadRegistry,
    private readonly activity: Activity = async () => {},
    private readonly options: ChatOptions = {},
  ) {}

  async mention(event: Mention): Promise<void> {
    const receivedAt = performance.now();
    const now = this.now();
    if (
      event.workspace !== this.config.workspace ||
      this.stopping ||
      event.channel !== this.config.channel ||
      event.bot ||
      !event.user
    )
      return;
    const age = now - Number(event.ts) * 1000;
    if (
      !event.id ||
      !Number.isFinite(age) ||
      age > 300_000 ||
      age < -60_000 ||
      !/^\d+\.\d+$/.test(event.ts)
    )
      return;
    // Bare follow-ups belong to the finding conversation. Explicit recipients
    // exclude Lens unless Slack also identified an explicit Lens mention.
    if (
      event.addressed === false &&
      /<@[A-Z0-9]+(?:\|[^>]+)?>/.test(event.text)
    )
      return;
    for (const [id, at] of this.seen)
      if (now - at > 600_000) this.seen.delete(id);
    const message = `${event.workspace}:${event.channel}:${event.ts}`;
    if (this.seen.has(message)) return;
    this.seen.set(message, now);
    if (this.seen.size > 2000) this.seen.delete(this.seen.keys().next().value!);
    for (const [id, thread] of this.threads)
      if (now - thread.at > 3_600_000) this.threads.delete(id);
    const thread =
      event.thread && /^\d+\.\d+$/.test(event.thread) ? event.thread : event.ts;
    if (event.addressed === false && (!event.thread || thread === event.ts))
      return;
    const context = await within(
      event.thread
        ? this.registry?.read(thread) ?? Promise.resolve(null)
        : Promise.resolve(null),
      this.options.contextTimeoutMs ?? 5000,
    ).then(
      (finding) => ({ ok: true as const, finding }),
      () => ({ ok: false as const }),
    );
    if (context?.ok === false) {
      if (event.addressed !== false) {
        void this.activity(event, "failed").catch(() => {});
        await this.deliver(event.channel, thread, {
          title: "Evidence unavailable",
          summary:
            "I could not load this thread's finding context. Please try again shortly",
          sources: [],
        });
      }
      return;
    }
    const finding = context?.finding;
    if (this.stopping) return;
    if (event.addressed === false && !finding) return;
    if (finding?.handled.includes(event.ts)) return;
    this.requests = this.requests.filter((at) => now - at < 86_400_000);
    if (
      this.busy ||
      this.requests.length >= 100 ||
      this.requests.filter((at) => now - at < 3_600_000).length >= 20
    ) {
      await this.deliver(event.channel, thread, {
        title: "Lens is busy",
        summary:
          "Lens is at its chat request limit or answering another question. Please try again later",
        sources: [],
      });
      return;
    }
    this.busy = true;
    this.requests.push(now);
    const controller = new AbortController();
    const done = this.answer(
      event,
      thread,
      finding ?? undefined,
      controller,
      receivedAt,
    );
    this.active = { controller, done };
    try {
      await done;
    } finally {
      this.busy = false;
      if (this.active?.controller === controller) this.active = undefined;
    }
  }

  async shutdown(graceMs = 8000): Promise<void> {
    this.stopping = true;
    const active = this.active;
    if (!active) return;
    active.controller.abort(new RequestEnded("shutdown"));
    await within(active.done, graceMs).catch(() => {});
  }

  private deliver(
    channel: string,
    thread: string,
    answer: Answer,
    message?: string,
    parent?: AbortSignal,
  ): Promise<void> {
    return withTimeout((signal) => {
      const active = parent ? AbortSignal.any([parent, signal]) : signal;
      active.throwIfAborted();
      return waitFor(
        this.reply(channel, thread, answer, message, active),
        active,
      );
    }, this.options.deliveryTimeoutMs ?? 10_000);
  }

  private async answer(
    event: Mention,
    thread: string,
    finding: FindingThread | undefined,
    controller: AbortController,
    receivedAt: number,
  ): Promise<void> {
    const timeout = setTimeout(
      () => controller.abort(new RequestEnded("timeout")),
      this.options.timeoutMs ?? 90_000,
    );
    let message: string | undefined;
    let acknowledgementStarted = false;
    let activityStarted = false;
    let acknowledgmentMs: number | null = null;
    let outcome = "error";
    let state: "done" | "failed" = "failed";
    let cleanup: Promise<void> | undefined;
    const finishActivity = () => {
      if (activityStarted && !cleanup)
        cleanup = within(
          this.activity(event, state),
          this.options.cleanupTimeoutMs ?? 5000,
        ).catch(() => {});
      return cleanup;
    };
    const input = redact(
      event.text
        .replace(/<@[A-Z0-9]+>/g, "")
        .trim()
        .slice(0, 2000),
    );
    const history: readonly Turn[] = [
      ...(finding?.turns ?? this.threads.get(thread)?.turns ?? []),
      { role: "user", content: input },
    ];
    try {
      if (
        finding &&
        !(await waitFor(
          within(
            this.registry!.claim(thread, event.ts),
            this.options.contextTimeoutMs ?? 5000,
          ),
          controller.signal,
        ))
      )
        return;
      controller.signal.throwIfAborted();
      const acknowledgement = this.options.acknowledge?.(event.channel, thread);
      acknowledgementStarted = Boolean(acknowledgement);
      activityStarted = true;
      void this.activity(event, "working").catch(() => {});
      if (acknowledgement) {
        message = await waitFor(
          within(acknowledgement, this.options.deliveryTimeoutMs ?? 10_000),
          controller.signal,
        );
        if (!/^\d+\.\d+$/.test(message))
          throw new Error("Invalid acknowledgment timestamp");
        acknowledgmentMs = Math.round(performance.now() - receivedAt);
      }
      controller.signal.throwIfAborted();
      const output = await waitFor(
        this.respond(history, controller.signal, finding ?? undefined),
        controller.signal,
      );
      await this.deliver(
        event.channel,
        thread,
        output,
        message,
        controller.signal,
      );
      state = "done";
      outcome = "done";
      const assistantTurn: Turn = {
        role: "assistant",
        content: output.opportunities?.length
          ? JSON.stringify(output.opportunities).slice(0, 3000)
          : `${output.title}\n${output.summary}`,
      };
      const turns = [...history, assistantTurn].slice(-6);
      if (finding)
        await within(this.registry!.remember(thread, turns), 2000).catch(() => {
          console.warn("Slack finding conversation could not be saved");
        });
      this.threads.set(thread, {
        at: this.now(),
        turns: turns.slice(-4),
      });
      if (this.threads.size > 100)
        this.threads.delete(this.threads.keys().next().value!);
    } catch (error) {
      void finishActivity();
      const failure = failureAnswer(error);
      outcome =
        error instanceof RequestEnded
          ? error.reason
          : failure.title === "Analysis blocked"
            ? "provider_quota"
            : "error";
      if (!acknowledgementStarted || message) {
        await this.deliver(event.channel, thread, failure, message).catch(
          () => {
            outcome = "delivery_error";
            console.warn("Slack completion could not be delivered");
          },
        );
      } else {
        outcome = "acknowledgment_error";
        console.warn(
          "Slack acknowledgment is incomplete; not retrying an ambiguous send",
        );
      }
    } finally {
      clearTimeout(timeout);
      await finishActivity();
      if (activityStarted || acknowledgementStarted)
        console.info(
          JSON.stringify({
            event: "lens_chat_completion",
            acknowledgment_ms: acknowledgmentMs,
            duration_ms: Math.round(performance.now() - receivedAt),
            outcome,
          }),
        );
    }
  }
}
