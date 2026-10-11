import { z } from "zod";
import { candidateSchema, verify } from "./findings.js";
import type { Evidence } from "./evidence.js";
import type { Repository } from "./repository.js";

export const documentationCandidateSchema = candidateSchema.extend({
  feedback_type: z.enum([
    "documentation_gap",
    "user_need",
    "capability_request",
    "none",
  ]),
  product_support: z.enum([
    "documented",
    "documented_limitation",
    "unverified",
  ]),
  documentation_gap: z.enum([
    "missing_instructions",
    "unclear_explanation",
    "outdated_example",
    "conflicting_guidance",
    "broken_link",
    "none",
  ]),
});

export const feedbackLabels = {
  documentation_gap: "Docs gap",
  user_need: "User need",
  capability_request: "Capability request",
} as const;

export const productSupportLabels = {
  documented: "Documented in the inspected sources",
  documented_limitation: "Limitation stated in the inspected sources",
  unverified: "Unverified; this finding does not establish product support",
} as const;

export const documentationScope = `This channel learns what users are trying to accomplish with the product from docs-search conversations. Focus on their workflows, unmet needs, requested capabilities, specific information they expect to find, and questions the docs leave unanswered. These instructions override the general agent-health categories and benchmark requirements above.
Choose feedback_type=documentation_gap for verified missing prerequisites or instructions, unclear explanations, outdated examples, conflicting guidance, or broken documentation links. Choose user_need for an actionable learning about a task or workflow readers want help accomplishing, even when no documentation defect has been established. Choose capability_request for explicit demand for a capability; this establishes demand, not that the capability is unsupported. User needs and capability requests use kind=opportunity. Use documentation_gap=none unless a concrete documentation defect was independently verified. Questions the agent could not answer are leads to investigate, not proof of missing docs.
Require an exact current human request, relevant current documentation sources actually read, and a concrete docs or product follow-up. For user needs, cite the nearest relevant guide as context, never as proof of absence. Set product_support=documented only when the cited docs explicitly support the requested workflow, documented_limitation only when they explicitly state its limitation, otherwise unverified. Lack of documentation never proves that the product cannot do something. Keep claims of unsupported functionality tied to explicit cited limitations. Use category Agent quality internally, but reader-facing titles about the user's task. Put what the inspected docs establish and what remains unknown in code_hypothesis. Put a specific edit, example, capability clarification, or product-validation question in experiment, with a practical check of the reader's desired outcome. No agent benchmark, frozen-case trial, latency or cost guardrail is required for these findings.
Before attributing a gap to the docs, inspect the relevant source page and plausible adjacent guides in the configured repository. Existing correct instructions that the search agent failed to retrieve, follow, summarize, or cite are not a documentation gap. Never infer missing documentation from an empty retrieval result, a model refusal, an uncited answer, an incomplete file read, or a bounded repository listing. State the exact inspected scope of a missing-content claim. Current source is not proof of what was deployed at the time of a trace.
Suppress model hallucinations, poor citation selection, retrieval or ranking failures, prompt or fallback bugs, gateway errors, token usage, latency, and other search-agent implementation problems. A generic "agent gave a bad answer" report is out of scope. Do not relabel those defects as user needs. A learning must add an actionable insight about the actual task beyond a routine, already-answered question. Single requests establish observed demand, never recurrence or prevalence; follow-ups within a conversation are not independent users. All source citations must be Markdown or MDX pages under docs/ or blog/. Use feedback_type=none, documentation_gap=none and kind=quiet when there is no supported, actionable docs or user-needs learning. Findings need no latency chart or performance claims.`;

export function verifyDocumentation(
  value: unknown,
  evidence: Evidence,
  repo: Pick<Repository, "files">,
) {
  const parsed = documentationCandidateSchema.safeParse(value);
  if (!parsed.success) return;
  const candidate = parsed.data;
  if (
    candidate.feedback_type === "none" ||
    (candidate.feedback_type === "documentation_gap" &&
      candidate.documentation_gap === "none") ||
    (candidate.feedback_type !== "documentation_gap" &&
      candidate.kind !== "opportunity") ||
    candidate.category !== "Agent quality" ||
    candidate.kind === "regression" ||
    candidate.frequency_metric !== "unknown" ||
    !candidate.evidence.some((item) => {
      const observation = evidence.observations.get(
        `${item.trace_id}:${item.span_id}`,
      );
      return (
        observation?.root &&
        observation.humanInput?.at(-1)?.includes(item.quote)
      );
    }) ||
    !candidate.code.every((item) =>
      /^(docs|blog)\/(?!.*(?:^|\/)\.\.(?:\/|$)).+\.mdx?$/.test(item.path),
    )
  )
    return;
  const verified = verify(candidate, evidence, repo);
  if (!verified) return;
  return {
    ...verified,
    feedback: {
      type: candidate.feedback_type,
      productSupport: candidate.product_support,
    },
  };
}
