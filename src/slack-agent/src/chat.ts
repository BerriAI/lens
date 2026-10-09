import type { Config } from "./config.js";
import type { Respond, Turn } from "./agent.js";
import type { Answer } from "./slack.js";

export interface Mention {
  readonly id: string;
  readonly workspace: string;
  readonly channel: string;
  readonly user: string;
  readonly text: string;
  readonly ts: string;
  readonly thread?: string;
  readonly bot?: boolean;
}
export type Reply = (
  channel: string,
  thread: string,
  answer: Answer,
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
    private readonly respond: Respond,
    private readonly reply: Reply,
    private readonly now: () => number = Date.now,
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
    for (const [id, at] of this.seen)
      if (now - at > 600_000) this.seen.delete(id);
    if (this.seen.has(event.id)) return;
    this.seen.set(event.id, now);
    if (this.seen.size > 2000) this.seen.delete(this.seen.keys().next().value!);
    for (const [id, thread] of this.threads)
      if (now - thread.at > 3_600_000) this.threads.delete(id);
    const thread =
      event.thread && /^\d+\.\d+$/.test(event.thread) ? event.thread : event.ts;
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
    const input = event.text
      .replace(/<@[A-Z0-9]+>/g, "")
      .trim()
      .slice(0, 2000);
    const history: readonly Turn[] = [
      ...(this.threads.get(thread)?.turns ?? []),
      { role: "user", content: input },
    ];
    try {
      const output = await this.respond(history, AbortSignal.timeout(90_000));
      await this.reply(event.channel, thread, output);
      this.threads.set(thread, {
        at: now,
        turns: [
          ...history,
          {
            role: "assistant" as const,
            content: output.opportunities
              ? JSON.stringify(output.opportunities).slice(0, 3000)
              : `${output.title}\n${output.summary}`,
          },
        ].slice(-4),
      });
      if (this.threads.size > 100)
        this.threads.delete(this.threads.keys().next().value!);
    } catch {
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
