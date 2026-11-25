"use client";

import type { ReactNode } from "react";
import { Checkbox } from "../../ui/checkbox";
import { Input } from "../../ui/input";
import { Textarea } from "../../ui/textarea";
import type { AgentIoDraft } from "./agentIo";

export function HttpAgentForm({
  draft,
  onChange,
}: {
  readonly draft: AgentIoDraft;
  readonly onChange: (change: Partial<AgentIoDraft>) => void;
}) {
  const text = (
    label: string,
    key: {
      [K in keyof AgentIoDraft]: string extends AgentIoDraft[K] ? K : never;
    }[keyof AgentIoDraft],
  ) => (
    <ApiField label={label}>
      <Input
        className="font-mono text-xs"
        value={draft[key]}
        onChange={(event) => onChange({ [key]: event.target.value })}
      />
    </ApiField>
  );
  return (
    <div className="grid gap-4">
      <p className="text-xs leading-5 text-muted-foreground">
        Lens saves this mapping. Its HTTP runner calls your agent from your
        machine or CI. After saving, set up the URL and authentication in your
        local connection profile
      </p>
      {text("Connection profile name", "connection")}
      <div className="grid gap-3 sm:grid-cols-2">
        {text("POST path", "submitPath")}
        {text("Accepted HTTP status", "acceptedStatus")}
      </div>
      <ApiField label="Request JSON">
        <Textarea
          className="min-h-28 font-mono text-xs"
          value={draft.body}
          onChange={(event) => onChange({ body: event.target.value })}
        />
      </ApiField>
      <p className="text-xs text-muted-foreground">
        Use fixed JSON with empty fields for inputs. Do not put credentials in
        this saved body. JSON Pointers such as /input or /messages/0/content
        select fields
      </p>
      {text("Case input pointer", "inputPointer")}
      <details className="space-y-3 text-xs">
        <summary className="cursor-pointer font-medium">
          Additional input mappings
        </summary>
        <div className="grid gap-3 sm:grid-cols-2">
          {text("Request ID pointer (optional)", "requestIdPointer")}
          {text("Follow-ups pointer (optional)", "followupsPointer")}
        </div>
        <p className="text-muted-foreground">
          Leave unused mappings blank. Follow-ups require an empty array in the
          request body
        </p>
      </details>
      <ApiField label="Response handling">
        <select
          className={SELECT_CLASS}
          value={draft.completion}
          onChange={(event) =>
            onChange({
              completion: event.target.value === "poll" ? "poll" : "immediate",
            })
          }
        >
          <option value="immediate">Returns the answer immediately</option>
          <option value="poll">
            Returns a job ID, then poll for the answer
          </option>
        </select>
      </ApiField>
      {draft.completion === "poll" && (
        <div className="grid gap-3 rounded-md border p-4">
          <div className="grid gap-3 sm:grid-cols-2">
            {text("Job ID pointer in accepted response", "idPointer")}
            {text("GET polling path", "pollPath")}
            {text("Status pointer", "statusPointer")}
            {text("Error pointer (optional)", "errorPointer")}
            {text("Success states (comma separated)", "success")}
            {text("Failure states (comma separated)", "failure")}
            {text("Poll interval (ms)", "intervalMs")}
            {text("Poll timeout (ms)", "timeoutMs")}
          </div>
          <details className="space-y-3 text-xs">
            <summary className="cursor-pointer font-medium">
              Require a completed result item
            </summary>
            {text("Result array pointer (optional)", "requiredArray")}
            <ApiField label="Required item fields (JSON)">
              <Textarea
                className="font-mono text-xs"
                value={draft.requiredMatches}
                onChange={(event) =>
                  onChange({ requiredMatches: event.target.value })
                }
              />
            </ApiField>
          </details>
        </div>
      )}
      {text("Output text pointer", "outputPointer")}
      <label className="flex items-center gap-2 text-xs">
        <Checkbox
          checked={draft.requireNonempty}
          onCheckedChange={(value) =>
            onChange({ requireNonempty: value === true })
          }
        />
        Require a nonempty answer
      </label>
      <label className="flex items-center gap-2 text-xs">
        <Checkbox
          checked={draft.trace}
          onCheckedChange={(value) => onChange({ trace: value === true })}
        />
        Link exported traces
      </label>
      {draft.trace ? (
        <div className="grid gap-3 rounded-md border p-4">
          <p className="text-xs text-muted-foreground">
            Your agent must already export traces to Lens with matching agent,
            version and environment attributes
          </p>
          <div className="grid gap-3 sm:grid-cols-2">
            <ApiField label="Trace ID response">
              <select
                className={SELECT_CLASS}
                value={draft.traceSource}
                onChange={(event) =>
                  onChange({
                    traceSource:
                      event.target.value === "completed"
                        ? "completed"
                        : "accepted",
                  })
                }
              >
                <option value="accepted">Accepted response</option>
                <option value="completed">Completed response</option>
              </select>
            </ApiField>
            <ApiField label="Trace attribute">
              <select
                className={SELECT_CLASS}
                value={draft.traceAttribute}
                onChange={(event) =>
                  onChange({
                    traceAttribute:
                      event.target.value === "trace_id"
                        ? "trace_id"
                        : "session.id",
                  })
                }
              >
                <option value="session.id">session.id</option>
                <option value="trace_id">trace_id</option>
              </select>
            </ApiField>
          </div>
          {text("Trace ID pointer", "tracePointer")}
        </div>
      ) : (
        <p className="text-xs text-muted-foreground">
          Without traces, an LLM judge scores the returned answer. Tool checks
          and cost gates need linked traces
        </p>
      )}
    </div>
  );
}

export const SELECT_CLASS =
  "h-9 w-full rounded-md border border-input bg-background px-3 text-xs";

export function ApiField({
  label,
  children,
}: {
  readonly label: string;
  readonly children: ReactNode;
}) {
  return (
    <label className="grid gap-1 text-xs">
      <span className="text-muted-foreground">{label}</span>
      {children}
    </label>
  );
}
