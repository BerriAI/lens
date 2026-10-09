"use client";

import { useState } from "react";
import { Github } from "lucide-react";
import { Button } from "../../../ui/button";
import { AgentGitHubDialog } from "../../agents/AgentGitHubDialog";
import { CodeBlock } from "../../onboarding/tracing/TracingSetupCard";
import { savedEvalTestSnippet } from "./connectSnippets";
import { SetupAgentPrompt } from "../../onboarding/SetupAgentPrompt";
import { useOptionalOnboarding } from "../../onboarding/OnboardingContext";
import type { EvalDefinition } from "./types";

export interface ConnectAgentProps {
  readonly definition: EvalDefinition;
  readonly dataset: string;
  readonly revision: number;
}

export function ConnectAgent({
  definition,
  dataset,
  revision,
}: ConnectAgentProps) {
  const [open, setOpen] = useState(false);
  const onboarding = useOptionalOnboarding();
  const canConnect = !onboarding?.readOnly;
  return (
    <section
      aria-label="Connect agent"
      className="mx-auto my-8 flex w-full max-w-3xl flex-col gap-5 px-6 text-sm"
    >
      <div>
        <h2 className="font-medium">No runs yet</h2>
        <p className="mt-1 text-sm leading-6 text-muted-foreground">
          This eval is saved. It uses cases from {dataset}, revision {revision}.
          Saving it does not run your agent
        </p>
      </div>
      <section
        className="space-y-3 rounded-md border p-4"
        aria-label="Run in your project"
      >
        <h3 className="font-medium">Run it in your project</h3>
        <p className="text-xs leading-5 text-muted-foreground">
          {definition.spec.agent_io
            ? "The saved HTTP contract maps each case to your agent's input and output. Configure its local connection profile before running"
            : "Write this Python test in your agent repository. Replace my_agent and the run call with your own entry point; return the actual output and trace ID"}
        </p>
        <p className="text-xs text-muted-foreground">
          Install the Lens SDK and set LENS_BASE_URL and LENS_API_KEY, plus your
          agent's model and tool credentials
        </p>
        <details>
          <summary className="cursor-pointer text-xs font-medium">
            Python test: tests/test_lens_eval.py
          </summary>
          <div className="mt-3">
            <CodeBlock
              code={savedEvalTestSnippet(definition)}
              copyLabel="Copy Python test"
            />
          </div>
        </details>
        <CodeBlock
          code="python -m pytest tests/test_lens_eval.py -v"
          copyLabel="Copy test command"
        />
        <a
          className="text-xs underline underline-offset-4"
          href="https://github.com/BerriAI/lens/tree/main/src/sdk#lens-evals"
          target="_blank"
          rel="noreferrer"
        >
          SDK setup guide
        </a>
      </section>
      <div className="flex flex-wrap items-center justify-between gap-3 border-b pb-5">
        <div>
          <h3 className="font-medium">Run in GitHub Actions</h3>
          <p className="mt-1 text-xs text-muted-foreground">
            Connect your repository and add the test to CI. Each run reports
            pass or fail here
          </p>
        </div>
        {canConnect ? (
          <Button variant="outline" size="sm" onClick={() => setOpen(true)}>
            <Github aria-hidden="true" className="size-4" /> Connect GitHub
          </Button>
        ) : (
          <p className="text-xs text-muted-foreground">
            Ask your Lens admin to connect this eval to GitHub
          </p>
        )}
      </div>
      <SetupAgentPrompt
        goal="evals"
        evaluation={{
          name: definition.name,
          agent: definition.spec.agent,
          dataset,
          revision,
        }}
      />
      {open && (
        <AgentGitHubDialog
          agent={definition.spec.agent}
          definition={definition}
          onOpenChange={setOpen}
        />
      )}
    </section>
  );
}
