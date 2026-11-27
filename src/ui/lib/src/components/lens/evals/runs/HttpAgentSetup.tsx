"use client";

import { useState } from "react";
import { Input } from "../../../ui/input";
import { CodeBlock } from "../../onboarding/tracing/TracingSetupCard";
import { ApiField, SELECT_CLASS } from "../HttpAgentForm";
import { workflowSnippet, type ConnectTarget } from "./connectSnippets";
import {
  agentOrigin,
  connectionProfile,
  namedRunCommand,
  type ConnectionAuth,
} from "./httpConnection";
import type { EvalDefinition } from "./types";

export function HttpAgentSetup({
  definition,
  workflowTarget,
}: {
  readonly definition: EvalDefinition;
  readonly workflowTarget?: ConnectTarget;
}) {
  const [origin, setOrigin] = useState("");
  const [auth, setAuth] = useState<ConnectionAuth>("bearer");
  const parsed = agentOrigin(origin);
  const io = definition.spec.agent_io;
  if (!io) return null;
  return (
    <section
      aria-label="HTTP API runner setup"
      className="min-w-0 space-y-4 text-sm"
    >
      <div className="space-y-1">
        <h3 className="font-medium">Connect your agent API</h3>
        <p className="text-xs leading-5 text-muted-foreground">
          The mapping is saved in Lens. The URL and authentication below
          generate a local connection profile. Copy it into your project; these
          connection settings are not stored in Lens
        </p>
      </div>
      <div className="grid gap-3 sm:grid-cols-2">
        <ApiField label="Agent origin">
          <Input
            type="url"
            placeholder="https://agent.example.com"
            value={origin}
            onChange={(event) => setOrigin(event.target.value)}
          />
        </ApiField>
        <ApiField label="Authentication">
          <select
            className={SELECT_CLASS}
            value={auth}
            onChange={(event) =>
              setAuth(event.target.value === "none" ? "none" : "bearer")
            }
          >
            <option value="bearer">Bearer API key</option>
            <option value="none">No authentication</option>
          </select>
        </ApiField>
      </div>
      {origin && !parsed.ok && (
        <p role="alert" className="text-xs text-destructive">
          {parsed.error}
        </p>
      )}
      <div className="space-y-2">
        <h4 className="text-xs font-medium">
          1. Add this profile to pyproject.toml
        </h4>
        <CodeBlock
          code={connectionProfile(io.connection, auth)}
          copyLabel="Copy connection profile"
        />
      </div>
      <div className="space-y-2 text-xs leading-5">
        <h4 className="font-medium">2. Set your runner environment</h4>
        <p className="text-muted-foreground">
          Set LENS_BASE_URL and LENS_API_KEY for Lens. Set AGENT_BASE_URL to
          your agent origin.
          {auth === "bearer" &&
            " Set AGENT_API_KEY through your shell or CI secret manager."}{" "}
          Set LENS_VERSION to the build deployed at that endpoint
        </p>
      </div>
      <div className="space-y-2">
        <h4 className="text-xs font-medium">3. Run the saved eval</h4>
        <p className="text-xs text-muted-foreground">
          Use a Lens SDK with saved HTTP contract support. The runner submits
          each case, collects its answer and reports results here
        </p>
        {(!origin || parsed.ok) && (
          <CodeBlock
            code={namedRunCommand(
              definition.name,
              parsed.ok ? parsed.value : undefined,
            )}
            copyLabel="Copy saved eval command"
          />
        )}
        <a
          className="text-xs underline"
          href="https://github.com/BerriAI/lens/tree/main/src/sdk#run-a-saved-eval"
          target="_blank"
          rel="noreferrer"
        >
          SDK installation and saved eval guide
        </a>
      </div>
      {workflowTarget && (
        <div className="space-y-2">
          <h4 className="text-xs font-medium">
            GitHub Actions: .github/workflows/lens.yml
          </h4>
          <p className="text-xs leading-5 text-muted-foreground">
            Add AGENT_BASE_URL, LENS_BASE_URL and LENS_VERSION as repository
            variables, and the API credentials as secrets. The runner must be
            able to reach both services. Deploy the intended agent build before
            running this workflow
          </p>
          <CodeBlock
            code={workflowSnippet({ ...workflowTarget, connectionAuth: auth })}
            copyLabel="Copy HTTP eval workflow"
          />
        </div>
      )}
      <details className="text-xs">
        <summary className="cursor-pointer font-medium">
          Saved request and response mapping
        </summary>
        <div className="mt-3">
          <CodeBlock
            code={JSON.stringify(io, null, 2)}
            copyLabel="Copy saved API mapping"
          />
        </div>
      </details>
    </section>
  );
}
