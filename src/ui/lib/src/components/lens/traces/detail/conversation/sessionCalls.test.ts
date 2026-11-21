import { describe, expect, it } from "vitest";
import research from "../../__fixtures__/research_trace.json";
import type { Span, SpanDetail, TraceMessage } from "../../types";
import { buildConversation, groupConversation, type ConversationGroup } from "./conversation";
import { compactedCalls, linkSessionCalls, newRequestMessages } from "./sessionCalls";
import { buildThread } from "./thread";

const base = {
  ...research.spans[0],
  parent_span_id: null,
  framework: "claude-code",
  status: "ok",
} as Span;
const system: TraceMessage = {
  role: "system",
  content: "You are a coding agent",
};
const ask = (content: string): TraceMessage => ({ role: "user", content });
const say = (content: string): TraceMessage => ({ role: "assistant", content });
const toolUse = say("");
const toolResult = ask("");

interface Recorded {
  span: Span;
  detail: SpanDetail;
}

let clock = 0;
const record = (id: string, shape: Partial<Span>, task: string, input: unknown, output: unknown): Recorded => ({
  span: {
    ...base,
    span_id: id,
    start_offset_ms: (clock += 10),
    duration_ms: 5,
    ...shape,
  },
  detail: {
    span_id: id,
    input: JSON.stringify(input),
    output: typeof output === "string" ? output : JSON.stringify(output),
    attributes: { "lens.task": task },
  },
});
const call = (id: string, task: string, request: TraceMessage[], reply: TraceMessage): Recorded =>
  record(id, { type: "llm", name: "chat model" }, task, request, [reply]);
const tool = (id: string, name: string, task: string, args: unknown, result: string): Recorded =>
  record(id, { type: "tool", name: `execute_tool ${name}` }, task, args, result);

function thread(recorded: readonly Recorded[]) {
  const spans = recorded.map(({ span }) => span);
  const details = new Map(recorded.map(({ span, detail }) => [span.span_id, detail]));
  return buildThread(groupConversation(buildConversation(spans, details, true), spans, details));
}

const texts = (messages: readonly TraceMessage[]): string[] => messages.map((message) => message.content);
const stepIds = (groups: readonly ConversationGroup[]): string[] =>
  groups.flatMap((group) => (group.kind === "item" ? [group.item.span.span_id] : stepIds(group.children)));

describe("newRequestMessages", () => {
  it("keeps what follows the last assistant message, or the whole request when there is none", () => {
    const first = [system, ask("one")];
    expect(newRequestMessages(first)).toEqual(first);
    expect(newRequestMessages([...first, say("1"), ask("two"), toolUse, toolResult])).toEqual([toolResult]);
    expect(newRequestMessages([...first, say("1"), ask("two"), ask("three")])).toEqual([ask("two"), ask("three")]);
  });
});

describe("gateway coding session threads", () => {
  it("shows each typed message once although every call resends the whole history", () => {
    const first = [system, ask("what is 17 times 3?")];
    const second = [...first, say("51"), ask("read the file")];
    const turns = thread([
      call("c1", "what is 17 times 3?", first, say("51")),
      call("c2", "what is 17 times 3?", second, {
        ...toolUse,
        tool_calls: [{ name: "Read", args: {} }],
      }),
      tool("t1", "Read", "what is 17 times 3?", { file_path: "a.md" }, "three lines"),
      call("c3", "what is 17 times 3?", [...second, toolUse, toolResult], say("It has 3 lines")),
    ]);
    expect(turns.map((turn) => [texts(turn.prompt), turn.reply?.content])).toEqual([
      [["what is 17 times 3?"], "51"],
      [["read the file"], "It has 3 lines"],
    ]);
    expect(texts(turns[0].context)).toEqual([system.content]);
    expect(turns[1].context).toEqual([]);
    expect([turns[1].llmCalls, turns[1].toolCalls]).toEqual([1, 1]);
  });

  it("puts a subagent's calls in a branch under the Agent call that launched it", () => {
    const main = [system, ask("list the files")];
    const prompt = "List every file in the folder";
    const sub = [{ role: "system", content: "You are a file search agent" }, ask(prompt)];
    const turns = thread([
      call("m1", "list the files", main, {
        ...toolUse,
        tool_calls: [{ name: "Agent", args: { prompt } }],
      }),
      call("s1", prompt, sub, {
        ...toolUse,
        tool_calls: [{ name: "Bash", args: { command: "ls" } }],
      }),
      tool("agent", "Agent", "list the files", { description: "List files", prompt }, "launched"),
      tool("bash", "Bash", prompt, { command: "ls" }, "a.md"),
      call("s2", prompt, [...sub, toolUse, toolResult], say("Only a.md")),
      call("m2", "list the files", [...main, toolUse, toolResult], say("The folder has a.md")),
    ]);
    expect(turns.map((turn) => texts(turn.prompt))).toEqual([["list the files"]]);
    expect(turns[0].reply?.content).toBe("The folder has a.md");
    const branches = turns[0].work.flatMap((work) => (work.kind === "subagent" ? [work] : []));
    expect(branches.map((branch) => [branch.name, stepIds(branch.groups)])).toEqual([
      ["List files", ["s1", "bash", "s2"]],
    ]);
    expect(turns[0].work.flatMap((work) => (work.kind === "step" ? [work.item.span.span_id] : []))).toEqual([
      "m1",
      "agent",
    ]);
  });

  it("keeps the main thread and one-off calls out of the branch when the subagent prompt repeats the user's message", () => {
    const typed = "list every file ".repeat(20).trim();
    const prompt = `${typed} and count them`;
    const main = [system, ask(typed)];
    const recorded = [
      call("m1", typed, main, { ...toolUse, tool_calls: [{ name: "Agent", args: { prompt } }] }),
      call("title", "write a title", [system, ask("write a title")], say("Files")),
      tool("agent", "Agent", typed, { description: "List files", prompt }, "launched"),
      call("s1", prompt, [system, ask(prompt)], say("Only a.md")),
      call("m2", typed, [...main, toolUse, toolResult], say("The folder has a.md")),
    ];
    const linked = linkSessionCalls(
      recorded.map(({ span }) => span),
      new Map(recorded.map(({ span, detail }) => [span.span_id, detail])),
    );
    expect(linked.map((span) => [span.span_id, span.parent_span_id])).toEqual([
      ["m1", null],
      ["title", null],
      ["agent", "m1"],
      ["s1", "agent"],
      ["m2", null],
    ]);
  });

  it("does not repeat the compaction summary as a typed message when the request has no assistant message", () => {
    const summary = ask("Summary of the earlier conversation");
    const turns = thread([
      call("c1", "first", [system, ask("first"), say("one"), ask("second")], say("two")),
      call("c2", "Summary", [system, summary, ask("third")], say("three")),
      call("c3", "Summary", [system, summary, ask("third"), say("three"), ask("fourth")], say("four")),
    ]);
    expect(turns.map((turn) => [turn.compacted, texts(turn.prompt)])).toEqual([
      [false, ["second"]],
      [true, ["third"]],
      [false, ["fourth"]],
    ]);
  });

  it("keeps a call in the main thread when no Agent call was given its task", () => {
    const turns = thread([
      call("m1", "first", [system, ask("first")], say("one")),
      tool("agent", "Agent", "first", { description: "Other", prompt: "something else" }, "launched"),
      call("x1", "unrelated", [system, ask("unrelated")], say("two")),
    ]);
    expect(turns.map((turn) => texts(turn.prompt))).toEqual([["first"], ["unrelated"]]);
    expect(turns.flatMap((turn) => turn.work).every((work) => work.kind === "step")).toBe(true);
  });

  it("marks a compaction and folds the summary instead of showing it as a typed message", () => {
    const before = [system, ask("first question"), say("first answer"), ask("second question")];
    const summary = ask("Summary of the earlier conversation");
    const turns = thread([
      call("c1", "first question", before, say("second answer")),
      call("c2", "Summary", [system, summary, say("second answer"), ask("third question")], say("third answer")),
      call(
        "c3",
        "Summary",
        [system, summary, say("second answer"), ask("third question"), say("third answer"), ask("fourth")],
        say("done"),
      ),
    ]);
    expect(turns.map((turn) => [turn.compacted, texts(turn.prompt), turn.reply?.content])).toEqual([
      [false, ["second question"], "second answer"],
      [true, ["third question"], "third answer"],
      [false, ["fourth"], "done"],
    ]);
    expect(texts(turns[1].context)).toEqual([summary.content]);
  });
});

describe("compactedCalls", () => {
  const main = [system, ask("first"), say("one"), ask("second")];
  const compactedRequest = [system, ask("summary"), say("two"), ask("third")];
  const titleRequest = [system, ask("write a title")];
  const calls = (...requests: TraceMessage[][]) =>
    requests.map((request, index) => ({
      id: `c${index}`,
      thread: "",
      request,
    }));

  it("reports the call whose history was replaced, with the message that replaced it", () => {
    expect([...compactedCalls(calls(main, compactedRequest))]).toEqual([["c1", ask("summary")]]);
  });

  it("ignores a one-off call that opens differently, wherever it falls", () => {
    expect(compactedCalls(calls(titleRequest, main, titleRequest, main, titleRequest)).size).toBe(0);
    expect([...compactedCalls(calls(main, titleRequest, compactedRequest)).keys()]).toEqual(["c2"]);
  });

  it("does not compare calls of different threads", () => {
    const threads = [
      { id: "main", thread: "", request: main },
      { id: "sub", thread: "agent", request: compactedRequest },
    ];
    expect(compactedCalls(threads).size).toBe(0);
  });
});
