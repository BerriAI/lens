import { z } from "zod";

const pointer = z
  .string()
  .regex(/^(?:\/(?:[^~/]|~[01])*)*$/, "Use a JSON Pointer such as /output");
const path = z
  .string()
  .refine(
    (value) =>
      value.startsWith("/") &&
      !value.startsWith("//") &&
      !/[?#{\\}\u0000-\u001f\u007f]/.test(value),
    "Use an absolute path on the agent origin, such as /invoke",
  );
const states = z
  .array(z.string().refine((value) => value.trim().length > 0))
  .min(1);
const scalar = z.union([z.string(), z.number(), z.boolean(), z.null()]);

export const agentIoSchema = z
  .object({
    version: z.literal(1),
    connection: z
      .string()
      .regex(
        /^[a-zA-Z][a-zA-Z0-9_-]{0,63}$/,
        "Connection name must start with a letter and contain only letters, digits, - or _",
      ),
    submit: z.object({
      method: z.literal("POST"),
      path,
      accepted_status: z.number().int().min(200).max(299),
      json: z.record(z.string(), z.json()),
    }),
    input: z
      .array(
        z.object({
          source: z.enum(["case.input", "case.followups", "trial.request_id"]),
          target: pointer,
        }),
      )
      .min(1),
    completion: z.discriminatedUnion("kind", [
      z.object({ kind: z.literal("immediate") }),
      z.object({
        kind: z.literal("poll"),
        id_pointer: pointer,
        path_template: z
          .string()
          .refine(
            (value) =>
              value.split("{id}").length === 2 &&
              path.safeParse(value.replace("{id}", "id")).success,
            "Polling path must contain exactly one {id}, such as /runs/{id}",
          ),
        status_pointer: pointer,
        success: states,
        failure: states,
        error_pointer: pointer.optional(),
        interval_ms: z.number().int().min(250).max(60_000),
        timeout_ms: z.number().int().min(1000).max(3_600_000),
        require_item: z
          .object({
            array_pointer: pointer,
            matches: z
              .record(z.string(), scalar)
              .refine(
                (value) => Object.keys(value).length > 0,
                "Supply at least one field to match",
              ),
          })
          .optional(),
      }),
    ]),
    output: z.object({ pointer, require_nonempty: z.boolean() }),
    trace: z
      .object({
        source: z.enum(["accepted", "completed"]),
        attribute: z.enum(["session.id", "trace_id"]),
        pointer,
      })
      .optional(),
  })
  .superRefine((io, context) => {
    const issue = (message: string) =>
      context.addIssue({ code: "custom", message });
    if (!io.input.some((binding) => binding.source === "case.input"))
      issue("Map the case input to a request field");
    if (
      io.input.some((binding, index) =>
        io.input
          .slice(0, index)
          .some(
            (other) =>
              binding.target === other.target ||
              binding.target.startsWith(`${other.target}/`) ||
              other.target.startsWith(`${binding.target}/`),
          ),
      )
    )
      issue("Input mappings must target separate, non-overlapping fields");
    if (
      io.input.some((binding) => {
        const value = resolvePointer(io.submit.json, binding.target);
        return (
          value === undefined ||
          (value !== null &&
            typeof value === "object" &&
            (!Array.isArray(value) || value.length > 0))
        );
      })
    )
      issue(
        "Every input target must exist in the request JSON and select a scalar or empty array",
      );
    if (io.completion.kind === "poll") {
      const poll = io.completion;
      const combined = [...poll.success, ...poll.failure];
      if (new Set(combined).size !== combined.length)
        issue("Success and failure states must be unique and must not overlap");
      if (poll.interval_ms > poll.timeout_ms)
        issue("Polling interval cannot exceed its timeout");
    }
  });

export type AgentIo = z.infer<typeof agentIoSchema>;

function resolvePointer(value: unknown, target: string): unknown {
  if (target === "") return value;
  return target
    .slice(1)
    .split("/")
    .reduce<unknown>((current, part) => {
      const key = part.replaceAll("~1", "/").replaceAll("~0", "~");
      if (
        current === null ||
        typeof current !== "object" ||
        !Object.hasOwn(current, key)
      )
        return undefined;
      if (Array.isArray(current) && !/^(0|[1-9]\d*)$/.test(key))
        return undefined;
      return Reflect.get(current, key);
    }, value);
}

export interface AgentIoDraft {
  readonly connection: string;
  readonly submitPath: string;
  readonly acceptedStatus: string;
  readonly body: string;
  readonly inputPointer: string;
  readonly requestIdPointer: string;
  readonly followupsPointer: string;
  readonly completion: "immediate" | "poll";
  readonly idPointer: string;
  readonly pollPath: string;
  readonly statusPointer: string;
  readonly success: string;
  readonly failure: string;
  readonly errorPointer: string;
  readonly intervalMs: string;
  readonly timeoutMs: string;
  readonly requiredArray: string;
  readonly requiredMatches: string;
  readonly outputPointer: string;
  readonly requireNonempty: boolean;
  readonly trace: boolean;
  readonly traceSource: "accepted" | "completed";
  readonly traceAttribute: "session.id" | "trace_id";
  readonly tracePointer: string;
}

export const EMPTY_AGENT_IO: AgentIoDraft = {
  connection: "agent-service",
  submitPath: "/invoke",
  acceptedStatus: "200",
  body: '{\n  "input": "",\n  "request_id": ""\n}',
  inputPointer: "/input",
  requestIdPointer: "/request_id",
  followupsPointer: "",
  completion: "immediate",
  idPointer: "/id",
  pollPath: "/runs/{id}",
  statusPointer: "/status",
  success: "completed",
  failure: "failed, cancelled",
  errorPointer: "",
  intervalMs: "1000",
  timeoutMs: "600000",
  requiredArray: "",
  requiredMatches: "{}",
  outputPointer: "/output",
  requireNonempty: true,
  trace: false,
  traceSource: "accepted",
  traceAttribute: "session.id",
  tracePointer: "/id",
};

export type AgentIoResult =
  | { readonly ok: true; readonly value: AgentIo }
  | { readonly ok: false; readonly error: string };

export function buildAgentIo(draft: AgentIoDraft): AgentIoResult {
  const parseJson = (text: string): unknown => {
    try {
      return JSON.parse(text);
    } catch {
      return undefined;
    }
  };
  const json = parseJson(draft.body);
  if (json === undefined)
    return { ok: false, error: "Request body must be valid JSON" };
  const result = agentIoSchema.safeParse({
    version: 1,
    connection: draft.connection.trim(),
    submit: {
      method: "POST",
      path: draft.submitPath.trim(),
      accepted_status: Number(draft.acceptedStatus),
      json,
    },
    input: [
      { source: "case.input", target: draft.inputPointer.trim() },
      ...(draft.requestIdPointer.trim()
        ? [
            {
              source: "trial.request_id",
              target: draft.requestIdPointer.trim(),
            },
          ]
        : []),
      ...(draft.followupsPointer.trim()
        ? [{ source: "case.followups", target: draft.followupsPointer.trim() }]
        : []),
    ],
    completion:
      draft.completion === "immediate"
        ? { kind: "immediate" }
        : {
            kind: "poll",
            id_pointer: draft.idPointer.trim(),
            path_template: draft.pollPath.trim(),
            status_pointer: draft.statusPointer.trim(),
            success: draft.success.split(",").map((value) => value.trim()),
            failure: draft.failure.split(",").map((value) => value.trim()),
            ...(draft.errorPointer.trim()
              ? { error_pointer: draft.errorPointer.trim() }
              : {}),
            interval_ms: Number(draft.intervalMs),
            timeout_ms: Number(draft.timeoutMs),
            ...(draft.requiredArray.trim()
              ? {
                  require_item: {
                    array_pointer: draft.requiredArray.trim(),
                    matches: parseJson(draft.requiredMatches),
                  },
                }
              : {}),
          },
    output: {
      pointer: draft.outputPointer.trim(),
      require_nonempty: draft.requireNonempty,
    },
    ...(draft.trace
      ? {
          trace: {
            source: draft.traceSource,
            attribute: draft.traceAttribute,
            pointer: draft.tracePointer.trim(),
          },
        }
      : {}),
  });
  if (!result.success) {
    const first = result.error.issues[0];
    return {
      ok: false,
      error: `${first.path.join(".") || "API mapping"}: ${first.message}`,
    };
  }
  return { ok: true, value: result.data };
}
