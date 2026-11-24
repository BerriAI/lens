"use client";

import { useState } from "react";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import type {
  McpConnection,
  McpCreateRequest,
  McpOverview,
} from "../../../lib/http/worker";
import { CodeBlock } from "../onboarding/tracing/TracingSetupCard";
import {
  AGENT_INSTRUCTIONS,
  CLIENTS,
  STARTER_PROMPT,
  installSnippet,
  tokenVariable,
} from "./model";

export const SELECT_CLASS =
  "h-9 rounded-md border border-input bg-background px-3 text-sm focus-visible:outline-ring";

export function McpSetup({
  overview,
  selected,
  endpoint,
  secret,
  onCreate,
  onWatch,
  watching,
  status,
  pending,
}: {
  overview: McpOverview;
  selected?: McpConnection;
  endpoint: string | null;
  secret: string | null;
  onCreate: (request: McpCreateRequest) => Promise<void>;
  onWatch: () => void;
  watching: boolean;
  status: string;
  pending: boolean;
}) {
  const [client, setClient] = useState<string>("Codex");
  const [name, setName] = useState("My coding agent");
  const [owner, setOwner] = useState("");
  const [days, setDays] = useState(90);
  const [requestId] = useState(() => crypto.randomUUID());
  const [showSecret, setShowSecret] = useState(false);
  const auth = selected?.authentication ?? overview.authentication;
  const variable = tokenVariable(auth);
  const active =
    selected &&
    !selected.revoked_at &&
    Date.parse(selected.expires_at) > Date.now();
  return (
    <div className="max-w-4xl space-y-7 py-6">
      <div className="space-y-2">
        <h3 className="text-base font-semibold">
          Give your agent access to Lens
        </h3>
        <p className="text-sm text-muted-foreground">
          Read traces, find failures, and inspect recorded cost from your coding
          agent
        </p>
        <p className="text-sm">
          <span className="font-medium">Access:</span>{" "}
          {selected
            ? auth === "token"
              ? "All workspace traces"
              : "Connection owner’s current gateway permissions"
            : overview.scope}
          . All five tools are read-only
        </p>
        <p className="text-sm text-muted-foreground">
          {auth === "gateway"
            ? "Use your own LiteLLM key. Your current permissions are checked on every request. The key keeps its other gateway permissions."
            : "Create a dedicated read-only token. It can read every trace in this deployment and cannot change Lens settings. Owner labels help you recognize connections; they do not verify who is using a shared token."}
        </p>
      </div>
      {!selected &&
        (overview.can_create ? (
          <form
            className="space-y-4"
            onSubmit={(event) => {
              event.preventDefault();
              void onCreate({
                request_id: requestId,
                name,
                client,
                expires_in_days: days,
                ...(auth === "token" && owner.trim()
                  ? { owner_label: owner.trim() }
                  : {}),
              });
            }}
          >
            <div className="grid gap-4 sm:grid-cols-2">
              <label className="flex flex-col gap-2 text-sm font-medium">
                Agent app
                <select
                  className={SELECT_CLASS}
                  value={client}
                  onChange={(event) => setClient(event.target.value)}
                >
                  {CLIENTS.map((value) => (
                    <option key={value}>{value}</option>
                  ))}
                </select>
              </label>
              <label className="flex flex-col gap-2 text-sm font-medium">
                Connection name
                <Input
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                  required
                  maxLength={128}
                />
              </label>
              {auth === "token" && (
                <label className="flex flex-col gap-2 text-sm font-medium">
                  Owner label (optional)
                  <Input
                    value={owner}
                    onChange={(event) => setOwner(event.target.value)}
                    maxLength={128}
                    placeholder="For example, Platform team"
                  />
                </label>
              )}
              <label className="flex flex-col gap-2 text-sm font-medium">
                Expires after
                <select
                  className={SELECT_CLASS}
                  value={days}
                  onChange={(event) => setDays(Number(event.target.value))}
                >
                  <option value={7}>7 days</option>
                  <option value={30}>30 days</option>
                  <option value={90}>90 days</option>
                  <option value={365}>365 days</option>
                </select>
              </label>
            </div>
            <Button type="submit" disabled={pending || !name.trim()}>
              {pending ? "Creating…" : "Create connection"}
            </Button>
          </form>
        ) : (
          <p role="status" className="rounded-lg border p-4 text-sm">
            Your role can view connections but cannot create them. Ask a Lens
            administrator for access
          </p>
        ))}
      {selected && (
        <>
          <div role="status" className="rounded-lg border p-4">
            <p className="font-medium">
              {selected.name}: {status}
            </p>
            <p className="mt-1 text-sm text-muted-foreground">
              {status === "Connected"
                ? "Lens received a successful tool call in this time window"
                : active
                  ? "Run a Lens tool from your agent to verify the connection. Copying setup instructions does not verify it"
                  : "Create a new connection and update your agent’s configuration to restore access"}
            </p>
            {active && (
              <Button
                variant="outline"
                size="sm"
                className="mt-3"
                onClick={onWatch}
                disabled={watching}
              >
                {watching ? "Waiting for your agent…" : "Check for a tool call"}
              </Button>
            )}
          </div>
          {active && (
            <>
              <section className="space-y-3">
                <h3 className="font-semibold">1. Prepare your credential</h3>
                {secret ? (
                  <>
                    <p className="text-sm text-muted-foreground">
                      Save this token in your password manager or secret store
                      now. It is shown only in this session and will not be
                      available after leaving this page
                    </p>
                    <div className="flex flex-wrap items-center gap-2">
                      <Input
                        aria-label="New MCP token"
                        type={showSecret ? "text" : "password"}
                        value={secret}
                        readOnly
                        autoComplete="off"
                        className="font-mono"
                      />
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => setShowSecret(!showSecret)}
                      >
                        {showSecret ? "Hide token" : "Reveal token"}
                      </Button>
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() =>
                          void navigator.clipboard.writeText(secret)
                        }
                      >
                        Copy token
                      </Button>
                    </div>
                  </>
                ) : (
                  <p className="text-sm text-muted-foreground">
                    {auth === "gateway"
                      ? "Use an existing LiteLLM key belonging to your signed-in user. A key from another user cannot use this connection"
                      : "Use the token saved when you created this connection. If you lost it, revoke this connection and create a new one"}
                  </p>
                )}
                <p className="text-sm text-muted-foreground">
                  Set <code>{variable}</code> in the environment that launches
                  your agent. Keep it out of source files and prompts
                </p>
                <CodeBlock
                  code={`export ${variable}='<your saved ${auth === "gateway" ? "LiteLLM key" : "MCP token"}>'`}
                  copyLabel="Copy credential environment template"
                />
              </section>
              <section className="space-y-3">
                <h3 className="font-semibold">
                  2. Add Lens to {selected.client}
                </h3>
                {endpoint ? (
                  <>
                    <p className="break-all font-mono text-xs text-muted-foreground">
                      {endpoint}
                    </p>
                    <CodeBlock
                      code={installSnippet(selected.client, endpoint, auth)}
                      copyLabel="Copy MCP configuration"
                    />
                    <p className="text-sm text-muted-foreground">
                      {selected.client === "Cursor"
                        ? "Add this entry to .cursor/mcp.json, preserving your existing servers. Launch Cursor with the environment variable available"
                        : selected.client === "Claude Code"
                          ? "Run this in your project terminal. Claude Code expands and stores the header when you add the server, so update it when the credential changes"
                          : selected.client === "Other"
                            ? "Add these connection fields in your MCP client’s settings"
                            : "Run this in your project terminal, then reopen your agent so it loads Lens"}
                    </p>
                  </>
                ) : (
                  <p role="alert" className="text-sm">
                    {auth === "token"
                      ? "This token connects directly to Lens. Open standalone Lens to get the connection URL"
                      : "The public gateway address is unavailable. Ask your administrator to configure the gateway sign-in address before connecting"}
                  </p>
                )}
              </section>
              <section className="space-y-3">
                <h3 className="font-semibold">3. Ask your agent to use Lens</h3>
                <CodeBlock
                  code={STARTER_PROMPT}
                  wrap
                  copyLabel="Copy starter prompt"
                />
                <p className="text-sm text-muted-foreground">
                  If Lens returns no traces, the connection still works. Send a
                  run from your project, or check the agent and time range, then
                  try again
                </p>
              </section>
            </>
          )}
        </>
      )}
      <details className="border-t pt-4">
        <summary className="cursor-pointer text-sm font-medium">
          Reusable agent instructions
        </summary>
        <div className="mt-3 space-y-3">
          <p className="text-sm text-muted-foreground">
            Add these to your agent’s project instructions, such as AGENTS.md or
            CLAUDE.md
          </p>
          <CodeBlock
            code={AGENT_INSTRUCTIONS}
            wrap
            copyLabel="Copy agent instructions"
          />
        </div>
      </details>
    </div>
  );
}
