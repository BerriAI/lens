"use client";

import { useState } from "react";
import { Github } from "lucide-react";
import { Button } from "../../../ui/button";
import { AgentGitHubDialog } from "../../agents/AgentGitHubDialog";
import { SetupAgentPrompt } from "../../onboarding/SetupAgentPrompt";
import { useOptionalOnboarding } from "../../onboarding/OnboardingContext";
import type { EvalDefinition } from "./types";

export interface ConnectAgentProps {
  readonly definition: EvalDefinition;
  readonly dataset: string;
  readonly revision: number;
}

export function ConnectAgent({ definition, dataset, revision }: ConnectAgentProps) {
  const [open, setOpen] = useState(false);
  const onboarding = useOptionalOnboarding();
  const canConnect = !onboarding?.readOnly;
  return (
    <section
      aria-label="Connect agent"
      className="lens-empty-state mx-auto my-6 flex max-w-3xl flex-col items-center gap-4 rounded-xl border border-dashed px-6 py-10 text-sm"
    >
      <span
        aria-hidden="true"
        className="flex size-11 items-center justify-center rounded-xl border border-[var(--lens-violet)]/20 bg-[var(--lens-violet)]/10 text-[var(--lens-violet)]"
      >
        <Github className="size-5" />
      </span>
      <div className="text-center">
        <h3 className="font-semibold">No runs yet</h3>
        <p className="mt-1 text-sm text-muted-foreground">
          Connect the repository for {definition.spec.agent} to run {definition.name} on pull requests
        </p>
      </div>
      {canConnect ? (
        <Button onClick={() => setOpen(true)}>
          <Github aria-hidden="true" className="size-4" /> Connect GitHub
        </Button>
      ) : (
        <p className="text-sm text-muted-foreground">Ask your Lens admin to connect this eval to GitHub</p>
      )}
      <SetupAgentPrompt
        goal="evals"
        evaluation={{
          name: definition.name,
          agent: definition.spec.agent,
          dataset,
          revision,
        }}
      />
      {open && <AgentGitHubDialog agent={definition.spec.agent} definition={definition} onOpenChange={setOpen} />}
    </section>
  );
}
