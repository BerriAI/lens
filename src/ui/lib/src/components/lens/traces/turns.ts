import type { Span } from "./types";

const NATIVE_AGENT_TOOLS = ["Agent", "Task", "execute_tool Agent", "execute_tool Task"];
const PROMPT_ARGUMENT = /"prompt":\s*"/;
const MIN_PROMPT_CHARS = 20;
const TRUNCATED_PREVIEW_CHARS = 200;

export function isNativeAgent(span: Span): boolean {
  return span.framework === "claude-code" && span.type === "tool" && NATIVE_AGENT_TOOLS.includes(span.name);
}

export interface SessionTurn {
  id: string;
  index: number;
  /** What the user typed to start the turn; empty when the calls carry no content. */
  message: string;
  /** The turn's own calls in time order, then the subagent calls nested under them. */
  spans: Span[];
}

export interface SessionTree {
  /** The recorded spans, with each subagent call re-parented under the Agent call that launched it. */
  spans: Span[];
  turns: SessionTurn[];
}

const endOf = (span: Span): number => span.start_offset_ms + span.duration_ms;
const byStart = (a: Span, b: Span): number => a.start_offset_ms - b.start_offset_ms;

const lastEndedBefore = (calls: readonly Span[], at: number): Span | undefined =>
  calls.reduce<Span | undefined>(
    (best, call) => (endOf(call) <= at && (!best || endOf(call) > endOf(best)) ? call : best),
    undefined,
  );

/** The prompt an Agent call was given, still JSON-escaped, read from a tool preview that may be cut short. */
function launchPrompt(agent: Span): { text: string; whole: boolean } | null {
  const match = PROMPT_ARGUMENT.exec(agent.input_preview);
  if (!match) return null;
  const rest = agent.input_preview.slice(match.index + match[0].length);
  const close = /(?<!\\)(?:\\\\)*"/.exec(rest);
  if (close) return { text: rest.slice(0, close.index + close[0].length - 1), whole: true };
  return { text: rest.replace(/\\[^\\]{0,5}$/, ""), whole: false };
}

function startsWithPrompt(call: Span, prompt: { text: string; whole: boolean }): boolean {
  const task = JSON.stringify(call.input_preview).slice(1, -1);
  if (!task) return false;
  if (prompt.whole)
    return task === prompt.text || (task.length >= TRUNCATED_PREVIEW_CHARS && prompt.text.startsWith(task));
  return prompt.text.length >= MIN_PROMPT_CHARS && task.startsWith(prompt.text);
}

function mostCommon(values: readonly string[]): string {
  const counts = new Map<string, number>();
  for (const value of values) counts.set(value, (counts.get(value) ?? 0) + 1);
  return [...counts].sort((a, b) => b[1] - a[1])[0]?.[0] ?? "";
}

/** A side call (naming the session) runs beside the first call with its own prompt; the turn's message is the one its calls on the session's usual model repeat most. */
function typedMessage(own: readonly Span[], usualModel: string): string {
  const calls = own.filter((span) => span.type === "llm" && span.input_preview);
  const usual = calls.filter((span) => (span.model ?? "") === usualModel);
  return mostCommon((usual.length ? usual : calls).map((span) => span.input_preview));
}

/**
 * A gateway records every model call and tool call of a coding session as its own parentless span, so the
 * trace has no turns and no subagent nesting. Both are rebuilt here from what each span carries: a subagent's
 * model calls preview the prompt its Agent call was given, a tool call follows the model call that asked
 * for it, and a model call previews the message the user typed last, so a turn opens at the first call, made
 * while no other is running, that previews a message the turn before it did not. Calls that carry no content
 * open a turn when they follow another finished model call with no tool call between them. Any other trace
 * returns null.
 */
export function sessionTree(recorded: readonly Span[]): SessionTree | null {
  const roots = [...recorded].sort(byStart);
  const calls = roots.filter((span) => span.type === "llm");
  const flat = roots.every((span) => span.parent_span_id === null && (span.type === "llm" || span.type === "tool"));
  if (!flat || calls.length < 2) return null;

  const agents = roots.filter(isNativeAgent).flatMap((agent) => {
    const prompt = launchPrompt(agent);
    const launcher = lastEndedBefore(calls, agent.start_offset_ms);
    return prompt ? [{ agent, prompt, launcher }] : [];
  });
  const owner = new Map<string, string>();
  for (const call of calls) {
    const launched = agents
      .filter(
        ({ agent, prompt, launcher }) =>
          endOf(call) > agent.start_offset_ms &&
          call.input_preview !== launcher?.input_preview &&
          startsWithPrompt(call, prompt),
      )
      .sort(
        (a, b) =>
          Math.abs(a.agent.start_offset_ms - call.start_offset_ms) -
          Math.abs(b.agent.start_offset_ms - call.start_offset_ms),
      );
    if (launched[0]) owner.set(call.span_id, launched[0].agent.span_id);
  }
  for (const tool of roots) {
    if (tool.type !== "tool") continue;
    const asker = lastEndedBefore(calls, tool.start_offset_ms);
    const agent = asker && owner.get(asker.span_id);
    if (agent && agent !== tool.span_id) owner.set(tool.span_id, agent);
  }

  const spans = roots.map((span) => {
    const parent = owner.get(span.span_id);
    return parent ? { ...span, parent_span_id: parent } : span;
  });
  const nested = (id: string): Span[] =>
    spans.filter((span) => span.parent_span_id === id).flatMap((span) => [span, ...nested(span.span_id)]);
  const top = spans.filter((span) => span.parent_span_id === null);
  const turns: Span[][] = [];
  let asked = new Set<string>();
  let lastCallEnd = -Infinity;
  for (const span of top) {
    const previous = turns.at(-1)?.at(-1);
    const idle = lastCallEnd <= span.start_offset_ms;
    const typed = span.input_preview;
    const opens =
      !previous ||
      (span.type === "llm" && idle && (typed ? !asked.has(typed) : asked.size === 0 && previous.type === "llm"));
    if (opens) {
      turns.push([span]);
      asked = new Set();
    } else turns.at(-1)?.push(span);
    if (span.type !== "llm") continue;
    if (typed) asked.add(typed);
    lastCallEnd = Math.max(lastCallEnd, endOf(span));
  }
  const usual = mostCommon(top.filter((span) => span.type === "llm").map((span) => span.model ?? ""));
  return {
    spans,
    turns: turns.map((own, index) => ({
      id: `turn::${own[0].span_id}`,
      index: index + 1,
      message: typedMessage(own, usual),
      spans: [...own, ...own.flatMap((span) => nested(span.span_id))],
    })),
  };
}
