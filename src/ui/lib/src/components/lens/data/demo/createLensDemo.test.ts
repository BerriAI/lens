import { describe, expect, it, vi } from "vitest";
import { createLensDemo } from "./createLensDemo";
import { createLensDemoData } from "./fixtures";
import { evidenceTarget } from "../../model/findings";
import { createOutcomeDemo } from "../../outcomes/outcomesModel";
import { outcomeForTrace, outcomeTraceId } from "../../outcomes/outcomeTrace";

describe("Lens demo data", () => {
  it("opens an outcome session as a full trace with consistent spend and explicit artifact tool evidence", async () => {
    const now = Date.parse("2026-11-05T12:00:00Z");
    const outcome = createOutcomeDemo(now)[0];
    const services = createLensDemo(now);
    const trace = await services.traces.trace(outcomeTraceId(outcome));
    expect(trace.summary.start_time).toBe(outcome.startedAt);
    expect(trace.summary.spend).toBe(outcome.costUSD);
    expect(trace.summary.input_preview).toBe(outcome.title);
    expect(trace.summary.agent_names).toEqual([outcome.agent]);
    expect(trace.summary.duration_ms).toBe(Date.parse(outcome.finishedAt) - Date.parse(outcome.startedAt));
    const tool = trace.spans.find((span) => span.type === "tool");
    expect(tool?.name).toBe("github.create_pull_request");
    const detail = await services.traces.span(trace.summary.trace_id, tool!.span_id);
    expect(JSON.parse(detail.output)).toMatchObject({
      repository: outcome.repository,
      pull_request: { number: outcome.pullRequest?.id, state: "open" },
      issue: { number: outcome.issue?.id },
    });
    expect(detail.attributes["github.pull_request.number"]).toBe(String(outcome.pullRequest?.id));
    expect(detail.attributes["github.issue.number"]).toBe(String(outcome.issue?.id));
    expect(trace.spans.find((span) => span.type === "llm")?.spend).toBe(outcome.costUSD);
    expect((await services.traces.conversation(trace.summary.trace_id)).turns).toEqual([]);
    expect(services.traces.handoff(trace.summary.trace_id).text).toContain(outcome.repository);
    expect(outcomeForTrace(trace.summary.trace_id, now + 60_000, trace.summary.start_time)).toEqual(outcome);
  });

  it("keeps supplemental outcome traces out of the ordinary demo list and agent totals", async () => {
    const now = Date.now();
    const services = createLensDemo(now);
    const data = createLensDemoData(now);
    const list = await services.traces.list({ startMs: 0, endMs: now }, "");
    expect(list.data.map((trace) => trace.trace_id)).toEqual(data.runs.map((run) => run.trace.summary.trace_id));
    const agents = await services.traces.agents({ startMs: 0, endMs: now });
    expect(agents.find((agent) => agent.name === "release_agent")?.runs).toBe(
      data.runs.filter((run) => run.trace.summary.agent_names.includes("release_agent")).length,
    );
    const unknown = (1000 + data.outcomeRuns.length + 1).toString(16).padStart(32, "0");
    expect(outcomeForTrace(unknown, now)).toBeUndefined();
    expect(outcomeForTrace(data.runs[0].trace.summary.trace_id, now)).toBeUndefined();
    await expect(services.traces.trace(unknown)).rejects.toMatchObject({ status: 404 });
  });
  it("shows the original order request before its follow-up without including later turns", async () => {
    const api = createLensDemo().traces;
    const history = await api.conversation("00000000000000000000000000000001");
    expect(history.turns).toHaveLength(1);
    expect(history.turns[0].input).toContain("Can you help me track my order?");
    expect(history.turns[0].output).toContain("Of course. What is your order number?");
    const original = await api.trace(history.turns[0].trace_id);
    const followup = await api.trace("00000000000000000000000000000001");
    expect(original.summary.source).toEqual(followup.summary.source);
    expect((await api.conversation(original.summary.trace_id)).turns).toEqual([]);
    expect(history.next_cursor).toBeNull();
  });

  it("links every finding to the quoted original step and assessed run", () => {
    const data = createLensDemoData();
    for (const lens of data.lenses) {
      for (const job of lens.jobs) {
        for (const finding of job.findings ?? []) {
          for (const evidence of finding.evidence) {
            const target = evidenceTarget(evidence.execution_id);
            const run = data.runs.find(({ trace }) => trace.summary.trace_id === target?.id);
            const step = run?.details.find((span) => span.span_id === evidence.span_id);
            expect(step?.output).toContain(evidence.quote);
            expect(job.sample?.executions.map((execution) => execution.id)).toContain(evidence.execution_id);
            expect(
              job.assessments.find((assessment) => assessment.execution_id === evidence.execution_id)?.[
                finding.kind === "issue" ? "issue_checks" : "pattern_checks"
              ],
            ).toContain(finding.check_id);
          }
        }
      }
    }
  });

  it("keeps trace totals, timestamps, and agent names consistent", () => {
    const data = createLensDemoData();
    for (const { trace } of data.runs) {
      expect(trace.summary.span_count).toBe(trace.spans.length);
      expect(trace.summary.agent_names).toContain(trace.agents[0].name);
      expect(trace.summary.error_count).toBe(trace.spans.filter((span) => span.status === "error").length);
      for (const span of trace.spans) {
        expect(span.start_offset_ms + span.duration_ms).toBeLessThanOrEqual(trace.summary.duration_ms);
      }
    }
  });

  it("includes a long release review with unique steps, complete details and three failed checks", () => {
    const run = createLensDemoData().runs[6];
    const ids = new Set(run.trace.spans.map((span) => span.span_id));
    expect(run.trace.spans).toHaveLength(362);
    expect(ids.size).toBe(362);
    expect(run.trace.summary.error_count).toBe(3);
    expect(run.trace.summary.status).toBe("ok");
    for (const span of run.trace.spans) {
      if (span.parent_span_id) expect(ids.has(span.parent_span_id)).toBe(true);
      expect(run.details.find((detail) => detail.span_id === span.span_id)).toBeDefined();
    }
    expect(JSON.parse(run.details.at(-1)!.input)).toHaveLength(120);
    expect(run.details.at(-1)!.output).toContain("Hold the release");
  });

  it("filters time windows locally and rejects writes or unknown reads without network access", async () => {
    const network = vi.spyOn(globalThis, "fetch");
    const now = Date.now();
    const services = createLensDemo(now);
    const all = await services.traces.list({ startMs: 0, endMs: now }, "");
    const recent = await services.traces.list({ startMs: now - 3600_000, endMs: now }, "");
    expect(recent.data.length).toBeGreaterThan(0);
    expect(recent.data.length).toBeLessThan(all.data.length);
    expect(recent.data.every((trace) => Date.parse(trace.start_time) >= now - 3600_000)).toBe(true);
    const settings = services.lens.lenses().then((list) => list.lenses[0].settings);
    await expect(services.lens.saveLens(undefined, await settings)).rejects.toMatchObject({ status: 403 });
    await expect(services.lens.run("real-investigation", "job")).rejects.toMatchObject({ status: 404 });
    await expect(services.traces.trace("missing")).rejects.toMatchObject({ status: 404 });
    expect(network).not.toHaveBeenCalled();
    network.mockRestore();
  });
});

it("serves review pages locally with the same cursor contract as live runs", async () => {
  const api = createLensDemo().lens;
  const { lenses } = await api.lenses();
  const owner = lenses[0];
  const job = owner.jobs[0];
  const first = await api.reviews(owner.id, job.id, 0);
  expect(first.reviews).toHaveLength(job.reviewed);
  expect(first.reviews.some((review) => review.verdicts.some((verdict) => verdict.kind === "issue"))).toBe(true);
  expect(await api.reviews(owner.id, job.id, first.reviewed)).toEqual({ reviews: [], reviewed: first.reviewed });
  await expect(api.reviews(owner.id, "missing", 0)).rejects.toMatchObject({ status: 404 });
});
