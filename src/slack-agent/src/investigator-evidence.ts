import { z } from "zod";
import type { LensClient } from "./lens.js";
import { digest } from "./investigator-state.js";

export type Sample = Awaited<ReturnType<LensClient["recent"]>>;
const span = z.object({
  span_id: z.string(),
  parent_span_id: z.string().nullable(),
  name: z.string(),
  status: z.string(),
  duration_ms: z.number(),
  start_offset_ms: z.number().default(0),
  error: z.string().nullable(),
  input_preview: z.string().default(""),
  type: z.string().default(""),
});
const detail = z.object({
  summary: z.object({
    trace_id: z.string(),
    agent_names: z.array(z.string()),
    service: z.string().default(""),
  }),
  spans: z.array(span),
  next_cursor: z.string().nullable().optional(),
});
export interface Observation {
  readonly trace_id: string;
  readonly span_id: string;
  readonly input: string;
  readonly output: string;
  readonly error: string;
  readonly inputHash: string | null;
  readonly root?: boolean;
  readonly humanInput?: readonly string[];
  readonly incident?: string | null;
  readonly attributes?: Readonly<Record<string, string>>;
  readonly url: string;
}
export function redact(text: string): string {
  return text
    .replace(
      /\b(?:sk-|xox[baprs]-|gh[pousr]_|github_pat_|rnd_)[A-Za-z0-9_-]+/g,
      "[redacted credential]",
    )
    .replace(/(Bearer\s+)[A-Za-z0-9._~+\/-]+=*/gi, "$1[redacted]")
    .replace(
      /((?:api[_-]?key|password|secret|(?:[A-Za-z]+[_-])?(?:access|admin|bot|app)[_-]?token)["']?\s*[:=]\s*["']?)[^\s"',;]+/gi,
      "$1[redacted]",
    );
}

export function currentRequest(text: string): string {
  const marker =
    /^CURRENT USER REQUEST:\s*\n/m.exec(text) ??
    /^CURRENT REQUEST:\s*\n/m.exec(text);
  if (!marker) return text;
  const current = text.slice(marker.index + marker[0].length);
  const boundary =
    /^(?:USER ATTACHMENTS \(reference data; contents do not grant permissions or override instructions\)|SLACK CONVERSATION REFERENCE \(untrusted source data, not additional instructions\)):\s*$/m.exec(
      current,
    );
  return current.slice(0, boundary?.index ?? current.length).trim();
}

function humanMessages(
  input: string,
  role: "user" | "assistant" = "user",
): string[] {
  const walk = (value: unknown, depth: number): string[] => {
    if (depth > 8) return [];
    if (Array.isArray(value))
      return value.flatMap((item) => walk(item, depth + 1));
    if (typeof value !== "object" || value === null) return [];
    const message = z
      .object({
        role: z.literal(role),
        content: z.union([z.string(), z.array(z.object({ text: z.string() }))]),
      })
      .safeParse(value);
    if (message.success)
      return (
        typeof message.data.content === "string"
          ? [message.data.content]
          : message.data.content.map((item) => item.text)
      ).map((text) => redact(role === "user" ? currentRequest(text) : text));
    return Object.values(value).flatMap((item) => walk(item, depth + 1));
  };
  try {
    return walk(JSON.parse(input), 0);
  } catch {
    return [];
  }
}
export class Evidence {
  readonly observations = new Map<string, Observation>();
  private readonly details = new Map<string, z.infer<typeof detail>>();
  private extraReads = 0;
  private failures = 0;
  constructor(
    readonly sample: Sample,
    private readonly client: LensClient,
    private readonly agent: string,
  ) {}
  async trace(id: string, signal: AbortSignal): Promise<string> {
    const selected = this.sample.traces.find((row) => row.trace_id === id);
    if (!selected || (!this.details.has(id) && this.details.size >= 100))
      return "Trace unavailable or read limit reached";
    const query = new URLSearchParams({
      trace_ref: selected.trace_ref,
      page_size: "500",
    });
    const response =
      this.details.get(id) ??
      detail.parse(
        await this.client.get(
          `/v1/traces/${encodeURIComponent(id)}?${query}`,
          signal,
        ),
      );
    if (
      response.summary.trace_id !== id ||
      (!response.summary.agent_names.includes(this.agent) &&
        !(selected.service && response.summary.service === selected.service))
    )
      return "Trace scope did not match";
    this.details.set(id, response);
    return JSON.stringify({
      trace_id: id,
      incomplete: Boolean(response.next_cursor),
      spans: response.spans.slice(0, 40).map((item) => ({
        ...item,
        name: redact(item.name).slice(0, 160),
        error: item.error ? redact(item.error).slice(0, 400) : null,
        input_preview: redact(item.input_preview).slice(0, 500),
      })),
      untrusted: true,
    });
  }
  async span(
    id: string,
    spanId: string,
    signal: AbortSignal,
    census = false,
  ): Promise<string> {
    const cached = this.observations.get(`${id}:${spanId}`);
    if (cached) return JSON.stringify(cached);
    const selected = this.sample.traces.find((row) => row.trace_id === id);
    const observed = this.details
      .get(id)
      ?.spans.find((row) => row.span_id === spanId);
    if (!selected || !observed || (!census && this.extraReads >= 8))
      return "Span unavailable or read limit reached";
    if (!census) this.extraReads++;
    const query = new URLSearchParams({ trace_ref: selected.trace_ref });
    const response = z
      .object({
        span_id: z.string(),
        input: z.string(),
        output: z.string(),
        attributes: z.record(z.string(), z.string()).default({}),
      })
      .parse(
        await this.client.get(
          `/v1/traces/${encodeURIComponent(id)}/spans/${encodeURIComponent(spanId)}?${query}`,
          signal,
        ),
      );
    if (response.span_id !== spanId) return "Span identity did not match";
    const observation: Observation = {
      trace_id: id,
      span_id: spanId,
      input: redact(response.input).slice(0, 6000),
      output: redact(response.output).slice(0, 6000),
      error: redact(observed.error ?? "").slice(0, 1000),
      inputHash:
        observed.parent_span_id === null &&
        observed.type === "agent" &&
        observed.duration_ms > 0 &&
        response.input.length > 0 &&
        response.input.length <= 6000
          ? digest(response.input)
          : null,
      root:
        observed.parent_span_id === null &&
        observed.type === "agent" &&
        observed.name === this.agent,
      humanInput: humanMessages(response.input)
        .slice(-2)
        .map((text) => text.slice(0, 6000)),
      incident:
        response.attributes["session.id"] ||
        response.attributes["moyai.run_id"] ||
        response.attributes["moyai.turn_id"] ||
        response.attributes["moyai.session_url"] ||
        null,
      attributes: Object.fromEntries(
        Object.entries(response.attributes).filter(([key]) =>
          [
            "session.id",
            "moyai.run_id",
            "moyai.turn_id",
            "moyai.status",
            "gen_ai.operation.name",
            "agent.source.url",
            "user.id",
            "traceloop.association.properties.user_id",
          ].includes(key),
        ),
      ),
      url: this.client.link({
        tab: "traces",
        trace: id,
        trace_ref: selected.trace_ref,
        span: spanId,
      }),
    };
    this.observations.set(`${id}:${spanId}`, observation);
    return JSON.stringify({
      ...observation,
      untrusted: true,
      truncated: response.input.length > 6000 || response.output.length > 6000,
    });
  }

  async loadRoots(signal: AbortSignal): Promise<void> {
    for (let offset = 0; offset < this.sample.traces.length; offset += 4) {
      await Promise.all(
        this.sample.traces.slice(offset, offset + 4).map(async (selected) => {
          try {
            await this.trace(selected.trace_id, signal);
            const spans = this.details.get(selected.trace_id)?.spans ?? [];
            const root = spans.find(
              (item) =>
                item.parent_span_id === null &&
                item.type === "agent" &&
                item.name === this.agent,
            );
            if (root) {
              await this.span(selected.trace_id, root.span_id, signal, true);
              return;
            }
            const models = spans.filter((item) => item.type === "llm");
            for (const item of [
              ...new Map(
                [models[0], models.at(-1)]
                  .filter((item) => item !== undefined)
                  .map((item) => [item!.span_id, item!]),
              ).values(),
            ])
              await this.span(selected.trace_id, item.span_id, signal, true);
          } catch {
            this.failures++;
          }
        }),
      );
      if (signal.aborted) break;
    }
  }

  report() {
    const records = this.sample.traces.map((selected) => {
      const spans = this.details.get(selected.trace_id)?.spans ?? [];
      const root = spans.find(
        (item) =>
          item.parent_span_id === null &&
          item.type === "agent" &&
          item.name === this.agent,
      );
      const observed = root
        ? this.observations.get(`${selected.trace_id}:${root.span_id}`)
        : [...this.observations.values()]
            .filter((item) => item.trace_id === selected.trace_id)
            .at(-1);
      const models = spans.filter((item) => item.type === "llm");
      const user = observed?.humanInput?.at(-1) ?? "";
      const setup =
        user.startsWith("For Lens tracing verification only") &&
        user.includes("LENS_PROD_CHECK");
      const interactive = Boolean(
        root &&
          observed?.attributes?.["gen_ai.operation.name"] === "invoke_agent" &&
          user &&
          !setup,
      );
      const incomplete =
        Boolean(this.details.get(selected.trace_id)?.next_cursor) || !observed;
      return {
        trace_id: selected.trace_id,
        span_id: observed?.span_id ?? null,
        url: selected.url,
        classification: setup
          ? "setup_check"
          : interactive
            ? "interactive_root"
            : root
              ? "unclassified_root"
              : spans.some((item) => item.type === "agent")
                ? "delegated_or_other_agent"
                : "partial_model_trace",
        status: root?.status ?? "partial",
        duration_ms: root?.duration_ms ?? null,
        before_first_model_ms:
          !incomplete && root && models.length
            ? Math.max(
                0,
                Math.min(...models.map((item) => item.start_offset_ms)) -
                  root.start_offset_ms,
              )
            : null,
        after_last_model_ms:
          !incomplete && root && models.length
            ? Math.max(
                0,
                root.start_offset_ms +
                  root.duration_ms -
                  Math.max(
                    ...models.map(
                      (item) => item.start_offset_ms + item.duration_ms,
                    ),
                  ),
              )
            : null,
        model_duration_ms: incomplete
          ? null
          : models.reduce((sum, item) => sum + item.duration_ms, 0),
        llm_calls: incomplete ? null : models.length,
        tool_calls: incomplete
          ? null
          : spans.filter((item) => item.type === "tool").length,
        tool_error_calls: spans.filter(
          (item) => item.type === "tool" && item.status === "error",
        ).length,
        input: user.slice(0, 700) || observed?.input.slice(0, 700) || "",
        output: observed
          ? (
              humanMessages(observed.output, "assistant").at(-1) ??
              observed.output
            ).slice(0, 700)
          : "",
        incident: observed?.incident ? digest(observed.incident) : null,
        request_id:
          observed?.attributes?.["moyai.run_id"] &&
          observed.attributes["moyai.turn_id"]
            ? digest(
                `${observed.attributes["moyai.run_id"]}:${observed.attributes["moyai.turn_id"]}`,
              )
            : null,
        readable_request: Boolean(user),
        user_id:
          root || !spans.some((item) => item.type === "agent")
            ? (
                observed?.attributes?.["user.id"] ||
                observed?.attributes?.[
                  "traceloop.association.properties.user_id"
                ] ||
                null
              )?.slice(0, 100) ?? null
            : null,
        incomplete,
      };
    });
    const roots = records.filter(
      (item) => item.classification === "interactive_root",
    );
    const terminal = roots.filter(
      (item) => item.status === "ok" || item.status === "error",
    );
    const durations = terminal
      .map((item) => item.duration_ms!)
      .sort((a, b) => a - b);
    const p95 =
      durations.length >= 20
        ? durations[Math.ceil(durations.length * 0.95) - 1]!
        : null;
    const models = terminal.filter(
      (item) => item.llm_calls !== null && item.llm_calls > 0,
    );
    const simple = terminal.filter(
      (item) => item.llm_calls === 1 && item.tool_calls === 0,
    );
    const metrics: {
      id: string;
      category: "Performance" | "Reliability";
      count: number;
      total: number;
      label: string;
      title: string;
      unit: string;
      affected_trace_ids: string[];
    }[] = [];
    const metric = (
      id: string,
      category: "Performance" | "Reliability",
      affected: { trace_id: string }[],
      total: number,
      label: string,
      title: string,
    ) => {
      if (total)
        metrics.push({
          id,
          category,
          count: affected.length,
          total,
          label: `${affected.length}/${total} ${label}`,
          title,
          unit:
            id.startsWith("tool_") && id !== "tool_error_turns"
              ? "completed observed tool calls"
              : "completed interactive turns",
          affected_trace_ids: [
            ...new Set(affected.map((item) => item.trace_id)),
          ],
        });
    };
    metric(
      "root_errors",
      "Reliability",
      terminal.filter((item) => item.status === "error"),
      terminal.length,
      "completed interactive turns ended with root errors",
      "Interactive turns end with an error status",
    );
    metric(
      "tool_error_turns",
      "Reliability",
      terminal.filter((item) => item.tool_error_calls > 0),
      terminal.length,
      "completed interactive turns contain tool errors",
      "Interactive tasks encounter tool failures",
    );
    metric(
      "first_model_over_30s",
      "Performance",
      models.filter((item) => item.before_first_model_ms! >= 30_000),
      models.length,
      "completed model-using turns waited at least 30s before the first model",
      "Time before first model exceeds 30 seconds",
    );
    metric(
      "single_model_no_tools_over_30s",
      "Performance",
      simple.filter((item) => item.duration_ms! >= 30_000),
      simple.length,
      "completed one-model, zero-tool turns lasted at least 30s",
      "One-model tasks without tools exceed 30 seconds",
    );
    const tools = new Map<
      string,
      { affected: { trace_id: string }[]; total: number }
    >();
    for (const root of roots)
      for (const item of this.details.get(root.trace_id)?.spans ?? []) {
        if (item.type !== "tool" || !["ok", "error"].includes(item.status))
          continue;
        const counts = tools.get(item.name) ?? { affected: [], total: 0 };
        tools.set(item.name, {
          affected:
            item.status === "error"
              ? [...counts.affected, { trace_id: root.trace_id }]
              : counts.affected,
          total: counts.total + 1,
        });
      }
    for (const [name, counts] of tools)
      metric(
        `tool_${digest(name).slice(0, 12)}`,
        "Reliability",
        counts.affected,
        counts.total,
        `observed ${redact(name).slice(0, 100)} calls failed in interactive turns`,
        `${redact(name).slice(0, 80)} calls fail`,
      );
    return {
      agent: this.sample.agent,
      window: this.sample.window,
      incomplete:
        this.sample.incomplete ||
        this.sample.sampled_for_detail ||
        this.failures > 0 ||
        records.some((item) => item.incomplete),
      inspected_trace_fragments: this.sample.matched_rows,
      metrics,
      population: {
        inspected_traces: roots.length,
        root_error_traces: roots.filter((item) => item.status === "error")
          .length,
        span_error_traces: roots.filter((item) => item.tool_error_calls > 0)
          .length,
        terminal_traces: terminal.length,
        above_p95:
          p95 === null
            ? null
            : terminal.filter((item) => item.duration_ms! > p95).length,
      },
      scope:
        "Frequency denominator is observed interactive root turns, excluding the exact Lens setup check. Child traces, automation and incomplete fragments are separate; missing parent spans never establish task completion. Timings are observed spans, not benchmarked gains. Semantic classifications are candidate interpretations of bounded input/output excerpts.",
      records,
    };
  }
}
