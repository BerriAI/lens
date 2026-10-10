import type { Config } from "./config.js";
import type { Turn } from "@litellm/lens-agent/models";
import type { ReplyAgent } from "./reply.js";
import type { Answer } from "./slack.js";
import type { ThreadRegistry } from "./threads.js";
import { redact } from "@litellm/lens-agent/evidence";

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
) => Promise<void>;
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

  constructor(
    private readonly config: Pick<Config, "workspace" | "channel">,
    private readonly respond: ReplyAgent,
    private readonly reply: Reply,
    private readonly now: () => number = Date.now,
    private readonly registry?: ThreadRegistry,
    private readonly activity: Activity = async () => {},
  ) {}

  async mention(event: Mention): Promise<void> {
    const now = this.now();
    if (
      event.workspace !== this.config.workspace ||
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
    // Bare follow-ups belong to the finding conversation. A leading mention
    // addresses someone else unless Slack also identified an explicit Lens mention.
    if (
      event.addressed === false &&
      /^\s*<@[A-Z0-9]+(?:\|[^>]+)?>/.test(event.text)
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
    const context = await (
      event.thread ? this.registry?.read(thread) : Promise.resolve(null)
    )?.then(
      (finding) => ({ ok: true as const, finding }),
      () => ({ ok: false as const }),
    );
    if (context?.ok === false) {
      if (event.addressed !== false) {
        await this.activity(event, "failed").catch(() => {});
        await this.reply(event.channel, thread, {
          title: "Evidence unavailable",
          summary:
            "I could not load this thread's finding context. Please try again shortly",
          sources: [],
        });
      }
      return;
    }
    const finding = context?.finding;
    if (event.addressed === false && !finding) return;
    if (finding?.handled.includes(event.ts)) return;
    this.requests = this.requests.filter((at) => now - at < 86_400_000);
    if (
      this.busy ||
      this.requests.length >= 100 ||
      this.requests.filter((at) => now - at < 3_600_000).length >= 20
    ) {
      await this.reply(event.channel, thread, {
        title: "Lens is busy",
        summary:
          "Lens is at its chat request limit or answering another question. Please try again later",
        sources: [],
      });
      return;
    }
    this.busy = true;
    this.requests.push(now);
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
      if (finding && !(await this.registry!.claim(thread, event.ts))) return;
      await this.activity(event, "working").catch(() => {});
      const output = await this.respond(
        history,
        AbortSignal.timeout(90_000),
        finding ?? undefined,
      );
      await this.reply(event.channel, thread, output);
      const assistantTurn: Turn = {
        role: "assistant",
        content: output.opportunities?.length
          ? JSON.stringify(output.opportunities).slice(0, 3000)
          : `${output.title}\n${output.summary}`,
      };
      const turns = [...history, assistantTurn].slice(-6);
      if (finding)
        await this.registry!.remember(thread, turns).catch(() => {
          console.warn("Slack finding conversation could not be saved");
        });
      this.threads.set(thread, {
        at: now,
        turns: turns.slice(-4),
      });
      if (this.threads.size > 100)
        this.threads.delete(this.threads.keys().next().value!);
      await this.activity(event, "done").catch(() => {});
    } catch {
      await this.activity(event, "failed").catch(() => {});
      await this.reply(event.channel, thread, {
        title: "Evidence unavailable",
        summary:
          "I could not finish reading the evidence. Please try again later or open Lens; this is not evidence that the agent is healthy",
        sources: [],
      });
    } finally {
      this.busy = false;
    }
  }
}
