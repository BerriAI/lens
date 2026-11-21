import type { AnalysisModelInfo, SignalConfig } from "./types";

export const isDecisionsMode = (mode: string | null | undefined): boolean =>
  mode === "decisions" || mode === "evaluation";

export const signalsConfigured = (config: SignalConfig): boolean =>
  Boolean(config.model) && (config.signals?.length ?? 0) > 0;

export const systemOneModels = (details: readonly AnalysisModelInfo[]): AnalysisModelInfo[] =>
  details
    .filter((info) => isDecisionsMode(info.mode))
    .toSorted((a, b) => a.model_group.localeCompare(b.model_group));

export const chatJudgeModels = (details: readonly AnalysisModelInfo[]): AnalysisModelInfo[] =>
  details.filter((info) => info.mode === "chat").toSorted((a, b) => a.model_group.localeCompare(b.model_group));

export interface LibrarySignal {
  readonly id: string;
  readonly name: string;
  readonly summary: string;
  readonly question: string;
}

export const DEFAULT_SIGNAL_THRESHOLD = 0.95;

export const SIGNAL_LIBRARY: readonly LibrarySignal[] = [
  {
    id: "user_frustration",
    name: "User frustration",
    summary: "annoyed, complaining or giving up",
    question:
      "Does the user show frustration, annoyance or dissatisfaction with the agent in this run, for example complaints, irritated corrections, all caps, profanity, or giving up on the task? Count actual dissatisfaction expressed by this user about the agent or its service, including dry sarcasm or giving up because of the agent. Capitalization, neutral corrections, quoted complaints, and anger about outside events or one's own mistakes are not sufficient. A later successful answer does not erase a real complaint.",
  },
  {
    id: "missing_capability",
    name: "Missing capability",
    summary: "asked for something the agent can't do",
    question:
      "Does this run show a required action blocked by an absent capability: no integration, no required tool or dataset, or a standing permission limit that remains at the end? Require direct evidence that the needed facility does not exist or is disabled for this agent. A tool returning an error, denial, stale row, or empty field on one call is not enough: the capability exists and the operation or data failed. A clearly separate required facility can still be absent. Also exclude missing input, policy refusals, temporary or resolved limits, and cases where an accepted alternative completes the required action.",
  },
  {
    id: "repeated_request",
    name: "Repeated request",
    summary: "had to ask for the same thing again",
    question:
      "Did the user make the same concrete request in two distinct live user turns? Count an explicit retry or rephrasing that still seeks the same requested result, including after access was restored. Supplying a missing field, reporting that access is restored, or consenting to a proposed next step is not by itself another request. A change of target, a new follow-up, or reformatting a result already delivered as requested is not repetition. Quoted dialogue, duplicated history pasted into later model inputs, and automatic tool retries do not add user turns.",
  },
  {
    id: "asked_for_human",
    name: "Asked for a human",
    summary: "asks a person to participate",
    question: "Did the user ask for a human to participate in the actual task? Count a request for a live person, staff member, or human role to speak, take over, review, approve, monitor, receive a handoff, contact the user, or otherwise act on the user's behalf. It counts even if requested alongside an unrelated task, the handoff is queued asynchronously, no direct conversation is requested, or delivery fails. Exclude quoted or hypothetical dialogue, a negated request, a handoff suggested only by the assistant, and requests for other software agents.",
  },
  {
    id: "tool_failure",
    name: "Tool failure",
    summary: "a tool or step failed and stayed broken",
    question:
      "Does a tool call or step fail in this run with an error, exception or timeout that the agent does not recover from? Count a real unresolved tool or execution-step error. Recovery via an equivalent retry or alternative counts as recovered; a stale result that fails a freshness requirement does not. A separate side-effect failure still counts even if the main answer succeeded. Zero matches from a successful lookup, quoted logs, and reported failures in a hypothetical example are not errors in this run.",
  },
  {
    id: "refused_request",
    name: "Refused request",
    summary: "declined a reasonable ask",
    question: "Did the assistant refuse a reasonable, allowed, concrete requirement in the actual request? A refusal communicates that the assistant will not perform the allowed task, or cannot perform it due to a standing capability limit. Mere tool or data failures, deferrals that promise future work, wrong answers, silence, missing inputs, required approvals, and required safety checks are not refusals. A negative factual answer is not a refusal. Do not count refusing an optional method if the user authorized another method that fulfilled the same need.",
  },
  {
    id: "made_up_answer",
    name: "Made-up answer",
    summary: "fabricated facts or false claims of completion",
    question:
      "Is there concrete evidence of a fabricated specific fact or a false claim that an action was done? Answer yes only when you can point to a material asserted claim that contradicts the record, requires external verification the trace shows did not occur, or falsely reports completion. Check user-supplied text and all tool results before deciding. A direct quote, valid calculation or inference, an honest statement of limits, an optional offer or suggestion, and an incomplete or off-topic first answer are not fabrications. When the trace cannot establish fabrication, answer no.",
  },
  {
    id: "task_abandoned",
    name: "Task abandoned",
    summary: "run ended without what was asked",
    question: "Did the assistant fail to deliver a concrete allowed outcome required in the actual request by the end of the trace? First locate the required result, then look for it in the final answer or successful tool output. A result that was wrong in a requested field, missing, only promised, incomplete, or in a format the user cannot use counts. Do not score supporting detail or stray commentary as abandonment when the core result is present and correct. A valid correction, requested draft, approved alternative, requested handoff or queued job, or a genuinely necessary clarification completes the stage. The user's frustrated walkaway does not complete the task.",
  },
];
