import type { Span, SpanDetail, TraceMessage } from "../../types";
import { toolInput } from "../content/payload";

const NATIVE_AGENT_TOOLS = ["Agent", "Task", "execute_tool Agent", "execute_tool Task"];
const TASK = "lens.task";
const MIN_TRUNCATED_TASK_CHARS = 200;

export const CONTEXT_COMPACTED = "Context compacted";

export function isNativeAgent(span: Span): boolean {
  return span.framework === "claude-code" && span.type === "tool" && NATIVE_AGENT_TOOLS.includes(span.name);
}

function agentPrompt(detail: SpanDetail | undefined): string {
  const args = detail ? toolInput(detail.input, detail.input_ui) : undefined;
  const prompt = args && typeof args === "object" && "prompt" in args ? args.prompt : undefined;
  return typeof prompt === "string" ? prompt.trim() : "";
}

function startsTask(prompt: string, task: string | undefined): boolean {
  if (!task || !prompt) return false;
  return prompt === task || (task.length >= MIN_TRUNCATED_TASK_CHARS && prompt.startsWith(task));
}

function nearest(span: Span, candidates: readonly Span[]): Span | undefined {
  return candidates.reduce<Span | undefined>((best, candidate) => {
    const distance = Math.abs(candidate.start_offset_ms - span.start_offset_ms);
    return !best || distance < Math.abs(best.start_offset_ms - span.start_offset_ms) ? candidate : best;
  }, undefined);
}

/**
 * A gateway records every model call and tool call of a coding session as its own parentless span.
 * A subagent's calls carry the prompt its Agent tool call was given as their task, so they are
 * placed under that tool call, and the tool call under the model call that issued it.
 */
export function linkSessionCalls(spans: readonly Span[], details: ReadonlyMap<string, SpanDetail>): Span[] {
  const task = (span: Span): string | undefined => details.get(span.span_id)?.attributes[TASK];
  const agents = spans
    .filter((span) => span.parent_span_id === null && isNativeAgent(span))
    .map((span) => ({ span, prompt: agentPrompt(details.get(span.span_id)) }));
  if (!agents.length) return [...spans];
  const owner = new Map(
    spans.flatMap((span) => {
      if (span.parent_span_id !== null) return [];
      const launched = agents.filter((agent) => agent.span !== span && startsTask(agent.prompt, task(span)));
      const agent = nearest(
        span,
        launched.map((candidate) => candidate.span),
      );
      return agent ? [[span.span_id, agent.span_id] as const] : [];
    }),
  );
  const caller = (agent: Span): string | undefined =>
    nearest(
      agent,
      spans.filter(
        (span) =>
          span.parent_span_id === null &&
          span.type === "llm" &&
          span.start_offset_ms <= agent.start_offset_ms &&
          owner.get(span.span_id) === owner.get(agent.span_id),
      ),
    )?.span_id;
  const launchedAgents = new Set(owner.values());
  return spans.map((span) => {
    const parent = owner.get(span.span_id) ?? (launchedAgents.has(span.span_id) ? caller(span) : undefined);
    return parent ? { ...span, parent_span_id: parent } : span;
  });
}

/** What a call adds to its thread: the request after the last assistant message, or all of it when there is none. */
export function newRequestMessages(request: readonly TraceMessage[]): TraceMessage[] {
  return request.slice(request.findLastIndex((message) => message.role === "assistant") + 1);
}

export interface ThreadCall {
  id: string;
  thread: string;
  request: readonly TraceMessage[];
}

const startKey = (request: readonly TraceMessage[]): string | undefined =>
  request.find((message) => message.role !== "system")?.content;

/**
 * Calls whose request no longer opens with the conversation the thread had been carrying, in time order.
 * A one-off call (a title request) opens differently too, so both sides must be conversations that already
 * hold an assistant message, and the replaced one must never come back.
 */
export function compactedCalls(calls: readonly ThreadCall[]): ReadonlyMap<string, TraceMessage> {
  const keyed = calls.map((call) => ({ ...call, key: startKey(call.request) }));
  const continued = (index: number): boolean =>
    keyed.some(
      (other) =>
        other.thread === keyed[index].thread &&
        other.key === keyed[index].key &&
        other.request.some((message) => message.role === "assistant"),
    );
  return new Map(
    keyed.flatMap((call, index) => {
      const earlier = keyed.slice(0, index).findLastIndex((other) => other.thread === call.thread);
      const previous = keyed
        .slice(0, index)
        .findLastIndex((other, at) => other.thread === call.thread && other.key !== undefined && continued(at));
      const summary = call.request.find((message) => message.role !== "system");
      if (previous < 0 || earlier < 0 || !summary || keyed[previous].key === call.key || !continued(index)) return [];
      const returns = keyed
        .slice(index)
        .some((other) => other.thread === call.thread && other.key === keyed[previous].key);
      const alreadyCompacted = keyed
        .slice(previous + 1, index)
        .some((other) => other.thread === call.thread && other.key === call.key);
      return returns || alreadyCompacted ? [] : [[call.id, summary] as const];
    }),
  );
}
