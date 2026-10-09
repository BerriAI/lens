"use client";

import { useId, type ComponentProps, type ReactNode } from "react";
import { Button } from "../../ui/button";
import { cn } from "../../../lib/cva.config";
import type { LensReadiness } from "../hooks/useLensReadiness";
import { useOnboarding } from "./OnboardingContext";
import { OnboardingSteps } from "./OnboardingSteps";
import { useLensAccessToken } from "../data/LensServices";
import { useLensService } from "./tracing/TracingSetupCard";
import { SetupAgentPrompt } from "./SetupAgentPrompt";

export type OnboardingSetupProps = Omit<ComponentProps<"section">, "children"> & {
  state: LensReadiness;
  action?: ReactNode;
  includeTracing?: boolean;
};

export function OnboardingSetup({ state, action, includeTracing = true, className, ...props }: OnboardingSetupProps) {
  const { readOnly, canInvestigate } = useOnboarding();
  const titleId = useId();
  const connection = useLensService(useLensAccessToken());
  return (
    <section
      data-slot="onboarding-setup"
      aria-labelledby={titleId}
      className={cn("min-w-0 scroll-mt-4 outline-none", className)}
      {...props}
    >
      <div className="mb-6 flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 id={titleId} className="text-2xl font-semibold tracking-tight">
            Get Lens running
          </h2>
          <p className="mt-2 text-sm leading-6 text-muted-foreground">
            Each step checks your connection, so you’ll see when it’s working.
          </p>
        </div>
        {action}
      </div>
      <SetupAgentPrompt connection={connection.data} />
      <OnboardingSteps state={state} includeTracing={includeTracing} />
      {state.error && connection.data?.configured !== false && (
        <div role="alert" className="mt-4 flex flex-wrap items-center gap-3 text-sm text-destructive">
          <p>Could not check setup. {state.error}</p>
          <Button variant="outline" size="sm" onClick={state.refresh} disabled={state.checking}>
            Retry
          </Button>
        </div>
      )}
      {(readOnly || !canInvestigate) && (
        <p className="mt-4 text-sm text-muted-foreground">
          An administrator can configure automatic analysis.
        </p>
      )}
    </section>
  );
}
