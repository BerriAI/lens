import type { Answer } from "./slack.js";

export class RequestEnded extends Error {
  constructor(readonly reason: "timeout" | "shutdown") {
    super(reason);
  }
}

// AbortSignal alone cannot stop an SDK or transport that never settles.
export function waitFor<T>(work: Promise<T>, signal: AbortSignal): Promise<T> {
  return new Promise((resolve, reject) => {
    const abort = () => reject(signal.reason ?? new Error("Request aborted"));
    if (signal.aborted) abort();
    else signal.addEventListener("abort", abort, { once: true });
    work.then(
      (value) => {
        signal.removeEventListener("abort", abort);
        resolve(value);
      },
      (error) => {
        signal.removeEventListener("abort", abort);
        reject(error);
      },
    );
  });
}

export async function within<T>(
  work: Promise<T>,
  milliseconds: number,
): Promise<T> {
  return withTimeout(() => work, milliseconds);
}

export async function withTimeout<T>(
  work: (signal: AbortSignal) => Promise<T>,
  milliseconds: number,
): Promise<T> {
  const controller = new AbortController();
  const timeout = setTimeout(
    () => controller.abort(new Error("Operation timed out")),
    milliseconds,
  );
  try {
    return await waitFor(work(controller.signal), controller.signal);
  } finally {
    clearTimeout(timeout);
  }
}

function exhaustedQuota(error: unknown): boolean {
  const pending: unknown[] = [error];
  const seen = new Set<unknown>();
  for (let reads = 0; pending.length && reads < 12; reads++) {
    const value = pending.shift();
    if (!value || typeof value !== "object" || seen.has(value)) continue;
    seen.add(value);
    const item = value as Record<string, unknown>;
    if (
      [item.code, item.type].some(
        (code) =>
          code === "insufficient_quota" || code === "credit_balance_exhausted",
      )
    )
      return true;
    if (
      typeof item.message === "string" &&
      (item.status === 429 || /^(?:Error:\s*)?429\b/.test(item.message)) &&
      /\b(?:insufficient_quota|credit_balance_exhausted)\b/.test(
        item.message.slice(0, 2000),
      )
    )
      return true;
    pending.push(
      ...[item.error, item.cause, item.response, item.data].filter(
        (next) => next && typeof next === "object",
      ),
    );
  }
  return false;
}

export function failureAnswer(error: unknown): Answer {
  if (exhaustedQuota(error))
    return {
      title: "Analysis blocked",
      summary:
        "The configured model provider has no credits. No result was produced. Please restore its credits and try again.",
      sources: [],
    };
  if (error instanceof RequestEnded)
    return {
      title:
        error.reason === "timeout"
          ? "Analysis timed out"
          : "Analysis interrupted",
      summary:
        error.reason === "timeout"
          ? "I could not finish within the time limit. No result was produced. Please try again or open Lens."
          : "Lens is restarting, so this analysis was interrupted. No result was produced. Please try again shortly.",
      sources: [],
    };
  return {
    title: "Evidence unavailable",
    summary:
      "I could not finish reading the evidence. Please try again later or open Lens; this is not evidence that the agent is healthy",
    sources: [],
  };
}
