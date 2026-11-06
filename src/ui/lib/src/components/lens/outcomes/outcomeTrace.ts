import { createOutcomeDemo, type OutcomeSession } from "./outcomesModel";

const TRACE_OFFSET = 1000;
const SESSION_COUNT = 144;

export function outcomeTraceId(session: OutcomeSession): string {
  const match = /^demo-session-(\d{3})$/.exec(session.id);
  const number = match ? Number(match[1]) : 0;
  if (number < 1 || number > SESSION_COUNT)
    throw new Error("Unsupported demo outcome session");
  return (TRACE_OFFSET + number).toString(16).padStart(32, "0");
}

export function outcomeForTrace(
  traceId: string,
  now: number,
  startedAt?: string,
): OutcomeSession | undefined {
  if (!/^0{29}[0-9a-f]{3}$/.test(traceId)) return undefined;
  const index = Number.parseInt(traceId, 16) - TRACE_OFFSET - 1;
  if (index < 0 || index >= SESSION_COUNT) return undefined;
  const relative = createOutcomeDemo(0)[index];
  const start = startedAt ? Date.parse(startedAt) : Number.NaN;
  const anchor = Number.isFinite(start)
    ? start - Date.parse(relative.startedAt)
    : now;
  return createOutcomeDemo(anchor)[index];
}
