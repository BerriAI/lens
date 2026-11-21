import type { Span } from "../../types";

const UNMATCHED_REASON: Record<
  Exclude<NonNullable<Span["spend_match"]>, "matched">,
  string
> = {
  no_call_id:
    "This step records no usable call identifier or transport context",
  no_spend_log: "No matching spend log belongs to this run",
  incomplete_evidence:
    "This step does not account for every request in the call",
  ambiguous: "Recorded identifiers do not select one compatible spend log",
};

export const unmatchedReason = (span: Span): string | null =>
  span.spend_match && span.spend_match !== "matched"
    ? UNMATCHED_REASON[span.spend_match]
    : null;

export function CostNotMatched({ span }: { span: Span }) {
  const reason = unmatchedReason(span);
  if (span.spend_log_request_id != null || !reason) return null;
  return (
    <span
      className="inline-flex items-center gap-1 text-xs text-muted-foreground"
      title={reason}
    >
      Cost <span className="font-medium text-foreground">not matched</span>
    </span>
  );
}
