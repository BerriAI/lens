import { z } from "zod";
import { Evidence, redact } from "./evidence.js";
import { Repository } from "./repository.js";
import { digest } from "./state.js";
import type { MeasuredFrequency, NativeFinding } from "./models.js";

export const PROMPT_REVISION = "lens-investigator-v1";
export const candidateSchema = z.object({
  kind: z.enum(["quiet", "issue", "regression", "opportunity", "frustration"]),
  category: z.enum(["Performance", "Agent quality", "Reliability"]),
  confidence: z.number(),
  impact: z.number(),
  frequency_metric: z.string(),
  issue_key: z.string(),
  title: z.string(),
  observation: z.string(),
  code_hypothesis: z.string(),
  experiment: z.string(),
  outcome: z.string(),
  limitation: z.string(),
  evidence: z.array(
    z.object({ trace_id: z.string(), span_id: z.string(), quote: z.string() }),
  ),
  code: z.array(z.object({ path: z.string(), quote: z.string() })),
});
export type Candidate = z.infer<typeof candidateSchema>;
export interface VerifiedCandidate {
  readonly candidate: Candidate;
  readonly issueNumber?: number;
  readonly fingerprint: string;
  readonly evidenceHash: string;
  readonly evidenceLinks: readonly string[];
  readonly codeLinks: readonly string[];
  readonly frequency?: MeasuredFrequency;
  readonly receipts?: readonly {
    readonly user: string | null;
    readonly request: string;
    readonly behavior: string;
    readonly quote: string;
    readonly url: string;
  }[];
}

export interface PersistedCandidate extends VerifiedCandidate {
  readonly native: NativeFinding;
}

export function verify(
  candidate: Candidate,
  evidence: Evidence,
  repo: Pick<Repository, "files">,
): VerifiedCandidate | undefined {
  if (
    candidate.kind === "quiet" ||
    candidate.confidence < 0.85 ||
    candidate.confidence > 1 ||
    !Number.isInteger(candidate.impact) ||
    candidate.impact < 5 ||
    candidate.impact > 10 ||
    !/^[a-z0-9_-]{3,80}$/.test(candidate.issue_key)
  )
    return;
  if (
    !candidate.title.trim() ||
    candidate.title.length > 160 ||
    candidate.observation.length > 1000 ||
    candidate.code_hypothesis.length > 1000 ||
    candidate.experiment.length > 1000 ||
    candidate.outcome.length > 180 ||
    !candidate.outcome.trim() ||
    candidate.limitation.length > 500 ||
    candidate.evidence.length < 1 ||
    candidate.evidence.length > 4 ||
    candidate.code.length < 1 ||
    candidate.code.length > 2
  )
    return;
  const prose = [
    candidate.title,
    candidate.observation,
    candidate.code_hypothesis,
    candidate.experiment,
    candidate.outcome,
    candidate.limitation,
  ].join(" ");
  if (
    redact(prose) !== prose ||
    /\d\s*%|\b(?:improved|reduced|faster|cheaper)\s+by\b/i.test(prose)
  )
    return;
  const observations = candidate.evidence.map((item) =>
    evidence.observations.get(`${item.trace_id}:${item.span_id}`),
  );
  if (
    candidate.evidence.some(
      (item, index) =>
        item.quote.trim().length < 8 ||
        item.quote.length > 240 ||
        !observations[index] ||
        ![
          observations[index]!.input,
          observations[index]!.output,
          observations[index]!.error,
          observations[index]!.humanInput?.at(-1) ?? "",
        ].some((text) => text.includes(item.quote)) ||
        redact(item.quote) !== item.quote,
    )
  )
    return;
  const humanRequest = candidate.evidence.find((item, index) =>
    observations[index]?.humanInput?.at(-1)?.includes(item.quote),
  );
  const singleRequest =
    candidate.kind === "opportunity" &&
    candidate.category === "Agent quality" &&
    Boolean(humanRequest);
  if (
    candidate.kind !== "frustration" &&
    !singleRequest &&
    new Set(candidate.evidence.map((item) => item.trace_id)).size < 2
  )
    return;
  if (
    candidate.kind === "frustration" &&
    !candidate.evidence.some(
      (item, index) =>
        observations[index]?.humanInput?.at(-1)?.includes(item.quote) &&
        /\b(frustrat\w*|annoy\w*|useless|unacceptable|doesn['’]t work|still (?:broken|not working)|stop (?:doing|saying)|wast(?:ing|ed) my time)\b/i.test(
          item.quote,
        ),
    )
  )
    return;
  const incidents = candidate.evidence.map(
    (item) =>
      [...evidence.observations.values()].find(
        (observation) =>
          observation.trace_id === item.trace_id && observation.root,
      )?.incident,
  );
  if (
    candidate.kind !== "frustration" &&
    !singleRequest &&
    (incidents.some((item) => !item) || new Set(incidents).size < 2)
  )
    return;
  if (candidate.kind === "regression") {
    const hashes = observations.map((item) => item?.inputHash);
    const compared = evidence.sample.traces
      .filter((row) =>
        candidate.evidence.some((item) => item.trace_id === row.trace_id),
      )
      .sort((a, b) => a.start_time.localeCompare(b.start_time));
    const before = compared[0];
    const after = compared.at(-1);
    if (
      !before ||
      !after ||
      before.trace_id === after.trace_id ||
      hashes.some((hash) => !hash) ||
      new Set(hashes).size !== 1 ||
      !["ok", "error"].includes(before.status) ||
      !["ok", "error"].includes(after.status) ||
      before.start_time >= after.start_time ||
      !(
        (before.status === "ok" && after.status === "error") ||
        (before.duration_ms > 0 && after.duration_ms >= before.duration_ms * 2)
      )
    )
      return verify(
        {
          ...candidate,
          kind: "issue",
          limitation: `Regression is unverified; ${candidate.limitation}`.slice(
            0,
            500,
          ),
        },
        evidence,
        repo,
      );
  }
  const codeLinks: string[] = [];
  for (const item of candidate.code) {
    const file = repo.files.get(item.path);
    if (
      !file ||
      item.quote.trim().length < 8 ||
      item.quote.length > 400 ||
      !file.content.includes(item.quote) ||
      redact(item.quote) !== item.quote
    )
      return;
    const line = file.content
      .slice(0, file.content.indexOf(item.quote))
      .split("\n").length;
    codeLinks.push(`${file.url}#L${line}`);
  }
  const report = evidence.report();
  let frequency: MeasuredFrequency | undefined = report.metrics.find(
    (metric) =>
      metric.id === candidate.frequency_metric &&
      metric.category === candidate.category &&
      metric.title === candidate.title &&
      candidate.evidence.every((item) =>
        metric.affected_trace_ids.includes(item.trace_id),
      ),
  );
  if ((singleRequest || candidate.kind === "frustration") && humanRequest) {
    const requests = [
      ...new Map(
        report.records
          .filter(
            (record) =>
              ["interactive_root", "partial_model_trace"].includes(
                record.classification,
              ) &&
              record.request_id &&
              record.readable_request,
          )
          .map((record) => [record.request_id, record]),
      ).values(),
    ];
    const supporting = requests.filter((record) =>
      record.input.includes(humanRequest.quote),
    );
    if (supporting.length && requests.length)
      frequency = {
        count: supporting.length,
        total: requests.length,
        label: `${supporting.length}/${requests.length} readable current requests contain this exact quote (observed support, not prevalence)`,
        title: candidate.title,
        unit: "readable current requests",
        support: true,
      };
  }
  return {
    candidate,
    fingerprint: digest(
      `${candidate.kind}:${candidate.issue_key}:${candidate.code
        .map((item) => item.path)
        .sort()
        .join(",")}`,
    ),
    evidenceHash: digest(
      candidate.evidence
        .map((item) => `${item.trace_id}:${item.span_id}:${item.quote}`)
        .sort()
        .join("\n"),
    ),
    evidenceLinks: observations.map((item) => item!.url),
    codeLinks,
    frequency,
    receipts: candidate.evidence.map((item, index) => {
      const record = report.records.find(
        (record) => record.trace_id === item.trace_id,
      );
      const user = record?.user_id;
      return {
        user:
          user &&
          /^(?:U[A-Z0-9]{7,30}|[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,})$/.test(
            user,
          )
            ? user
            : null,
        request: record?.input ?? "",
        behavior: record?.output ?? "",
        quote: item.quote,
        url: observations[index]!.url,
      };
    }),
  };
}
