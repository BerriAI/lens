import type { Settings } from "./types";

export interface Watch {
  id: string;
  name: string;
  summary: string;
  instruction: string;
  defaultOn: boolean;
}

export const analysisDimensions: readonly Watch[] = [
  {
    id: "watch_performance",
    name: "performance",
    summary: "slow replies and time outside model calls",
    instruction:
      "Find runs with excessive latency relative to the requested work, including short replies delayed before or after model calls. Use recorded timing measurements and original task evidence. Distinguish time outside recorded model spans from a proven cause; long coding tasks are not short replies.",
    defaultOn: false,
  },
  {
    id: "watch_efficiency",
    name: "efficiency",
    summary: "oversized context and unnecessary usage",
    instruction:
      "Find runs where prompt size, repeated calls, cache use or recorded gateway cost is disproportionate to the task. Use supplied usage measurements and original task evidence. Missing cache or cost values are unknown, never zero; large context alone does not prove waste.",
    defaultOn: false,
  },
  {
    id: "watch_telemetry",
    name: "telemetry integrity",
    summary: "missing or inconsistent trace evidence",
    instruction:
      "Find observability gaps that prevent reliable analysis, such as missing messages despite recorded token usage, incomplete span trees or inconsistent telemetry. Distinguish recording limitations and intentional redaction from failed model calls. State unknown capture policy and do not infer missing cost should be available.",
    defaultOn: false,
  },
];

export const watches: readonly Watch[] = [
  {
    id: "watch_unsolved",
    name: "unsolved",
    summary: "didn't finish what was asked",
    instruction:
      "Find runs where the agent failed to solve what the user asked for: wrong or partial answers, giving up, or stopping mid-task.",
    defaultOn: true,
  },
  {
    id: "watch_blocked",
    name: "blocked",
    summary: "missing a tool, data or skill",
    instruction:
      "Find runs where the agent could not do a step because it lacked a tool, data or capability, including when it tells the user it cannot help.",
    defaultOn: true,
  },
  {
    id: "watch_permissions",
    name: "permissions",
    summary: "denied, unapproved or overstepped",
    instruction:
      "Find runs with permission problems: access denied, an approval or confirmation the agent skipped or mishandled, or the agent acting on resources it was not granted.",
    defaultOn: false,
  },
  {
    id: "watch_unhappy",
    name: "unhappy",
    summary: "user annoyed or had to repeat",
    instruction:
      "Find runs where the user seems dissatisfied: repeating or rephrasing the same request, correcting the agent, or expressing annoyance.",
    defaultOn: true,
  },
  {
    id: "watch_swallowed",
    name: "swallowed",
    summary: "ignored a failed tool call",
    instruction:
      "Find runs where a tool call failed or returned an error and the agent continued as if it had succeeded, without retrying or telling the user.",
    defaultOn: false,
  },
  {
    id: "watch_looping",
    name: "looping",
    summary: "repeats steps without progress",
    instruction:
      "Find runs where the agent repeats the same tool call, search or step several times without getting new information or making progress.",
    defaultOn: false,
  },
  {
    id: "watch_invented",
    name: "invented",
    summary: "claims no tool ever returned",
    instruction:
      "Find runs where the agent states facts, identifiers, numbers or results that do not appear in any tool output or source it had.",
    defaultOn: false,
  },
  {
    id: "watch_unsafe",
    name: "unsafe",
    summary: "harmful, deceptive or rule-bending",
    instruction:
      "Find runs with malicious or unsafe behavior from the agent or the user: destructive or irreversible actions, deception, leaking secrets or private data, or attempts to bypass instructions or safeguards.",
    defaultOn: false,
  },
  ...analysisDimensions,
];

export function watchChecks(enabled: ReadonlySet<string>): Settings["checks"] {
  return watches
    .filter((watch) => enabled.has(watch.id))
    .map(({ id, instruction }) => ({ id, instruction, enabled: true }));
}

export function initialWatches(checks: Settings["checks"] | undefined): ReadonlySet<string> {
  if (!checks?.length) return new Set(watches.filter((watch) => watch.defaultOn).map((watch) => watch.id));
  const ids = new Set(watches.map((watch) => watch.id));
  return new Set(checks.filter((check) => ids.has(check.id) && check.enabled).map((check) => check.id));
}

export function isWatch(check: Settings["checks"][number]): boolean {
  return watches.some((watch) => watch.id === check.id);
}

export function setAnalysisDimension(checks: Settings["checks"], watch: Watch, enabled: boolean): Settings["checks"] {
  return checks.some((check) => check.id === watch.id)
    ? checks.map((check) => (check.id === watch.id ? { ...check, enabled } : check))
    : [...checks, { id: watch.id, instruction: watch.instruction, enabled }];
}
