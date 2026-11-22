"use client";

import { useId, useRef, useState, type ReactNode } from "react";
import { ArrowRight, ChevronDown } from "lucide-react";
import { Button } from "../../ui/button";
import { TracingSetupFields, useLensService } from "./tracing/TracingSetupCard";
import { cn } from "../../../lib/cva.config";
import { useLensAccessToken } from "../data/LensServices";
import type { LensReadiness } from "../hooks/useLensReadiness";
import { initialSetupStep } from "../model/readiness";
import { StepIndicator, type StepState } from "../ui/StepIndicator";
import { useOnboarding } from "./OnboardingContext";

type StepProps = {
  state: LensReadiness;
  goTo: (step: number) => void;
  storageReady: boolean;
};

function useLocked() {
  const { readOnly, canInvestigate } = useOnboarding();
  return readOnly || !canInvestigate;
}

function StorageStep({ state, goTo, storageReady }: StepProps) {
  const { readOnly, openTrace } = useOnboarding();
  const accessToken = useLensAccessToken();
  if (!storageReady)
    return (
      <>
        <TracingSetupFields
          detail="Tracing is not enabled"
          accessToken={accessToken}
          onOpenTrace={openTrace}
          onCheck={state.refresh}
          checking={state.checking}
          readOnly={readOnly}
        />
        <ActivityContinuation state={state} />
      </>
    );
  return (
    <div className="space-y-4">
      <p role="status" className="text-sm text-success">
        Trace storage is connected
      </p>
      <Button onClick={() => goTo(1)}>
        Continue to your agent <ArrowRight aria-hidden="true" className="size-4" />
      </Button>
      <ActivityContinuation state={state} />
    </div>
  );
}

function ActivityContinuation({ state }: { state: LensReadiness }) {
  const { connect, create } = useOnboarding();
  const locked = useLocked();
  if (!state.activityReady) return null;
  return (
    <div className="mt-4">
      <Button onClick={state.connected ? create : connect} disabled={locked}>
        {state.connected ? "View automatic analysis" : "Configure analysis"}
        <ArrowRight aria-hidden="true" className="size-4" />
      </Button>
      {state.requestsReady && !state.tracesReady && (
        <p className="mt-2 text-xs text-muted-foreground">
          Request logs are already available. Connect agent traces to receive automatic findings.
        </p>
      )}
    </div>
  );
}

function AgentStep({ state }: StepProps) {
  const { readOnly, canMintTracingKey, openTrace } = useOnboarding();
  const accessToken = useLensAccessToken();
  return (
    <>
      <div hidden={state.tracesReady}>
        <TracingSetupFields
          detail={null}
          accessToken={accessToken}
          onOpenTrace={(value) => {
            state.refresh();
            openTrace(value);
          }}
          onCheck={state.refresh}
          checking={state.checking}
          readOnly={readOnly}
          canMintTracingKey={canMintTracingKey}
        />
      </div>
      {state.tracesReady && (
        <p role="status" className="text-sm text-success">
          Your first trace is ready. Continue setup so Lens can analyze your agent’s behavior.
        </p>
      )}
      <ActivityContinuation state={state} />
    </>
  );
}

function AnalysisStep({ state }: StepProps) {
  const { connect, create } = useOnboarding();
  const locked = useLocked();
  return (
    <div className="space-y-4">
      <p role="status" className="text-sm text-muted-foreground">
        {state.connected
          ? "Analysis is configured. Findings will appear after your agent sends 10 traces."
          : "Connect a model in Settings. Lens will analyze each agent automatically after 10 traces."}
      </p>
      {!state.activityReady && (
        <p className="text-sm text-muted-foreground">Send traces from your agent to start automatic analysis.</p>
      )}
      <Button onClick={state.connected ? create : connect} disabled={!state.activityReady || locked}>
        {state.connected ? "View automatic analysis" : "Configure analysis"}
        <ArrowRight aria-hidden="true" className="size-4" />
      </Button>
    </div>
  );
}

function InvestigationStep({ state }: StepProps) {
  const { create } = useOnboarding();
  const locked = useLocked();
  return (
    <div className="space-y-4">
      <p className="text-sm leading-6 text-muted-foreground">
        Lens analyzes new traces automatically. In Findings, choose a model, edit what to look for, and set how often to
        check. Each finding links to the evidence.
      </p>
      {!state.ready && (
        <p className="text-sm text-muted-foreground">
          Connect an analysis model and send 10 traces from your agent to receive findings.
        </p>
      )}
      <Button onClick={create} disabled={!state.ready || locked}>
        View automatic analysis <ArrowRight aria-hidden="true" className="size-4" />
      </Button>
    </div>
  );
}

interface StepDefinition {
  readonly title: string;
  readonly description: string;
  readonly complete: (state: LensReadiness, storageReady: boolean) => boolean;
  readonly Content: (props: StepProps) => ReactNode;
}

const STEPS: readonly StepDefinition[] = [
  {
    title: "Install Lens",
    description: "Enable Lens in your Helm or Docker deployment.",
    complete: (_state, storageReady) => storageReady,
    Content: StorageStep,
  },
  {
    title: "Send your first trace",
    description: "Capture your agent’s inputs, outputs, and tool calls.",
    complete: (state) => state.tracesReady,
    Content: AgentStep,
  },
  {
    title: "Configure analysis",
    description: "Choose an analysis model and spending limit.",
    complete: (state) => state.connected,
    Content: AnalysisStep,
  },
  {
    title: "Review findings",
    description: "Follow automatic analysis and review the results.",
    complete: (state) => state.hasInvestigations,
    Content: InvestigationStep,
  },
];

function stepState(complete: boolean, open: boolean): StepState {
  if (complete) return "complete";
  return open ? "current" : "upcoming";
}

export function OnboardingSteps({
  state,
  className,
  includeTracing,
}: {
  state: LensReadiness;
  className?: string;
  includeTracing: boolean;
}) {
  const id = useId();
  const listRef = useRef<HTMLOListElement>(null);
  const offset = includeTracing ? 0 : 2;
  const accessToken = useLensAccessToken();
  const service = useLensService(accessToken);
  const storageReady = Boolean(service.data?.connected && service.data.status.storage_ready && service.data.url);
  const [step, setStep] = useState(() => Math.max(offset, initialSetupStep(state)));
  const visibleStep = !storageReady && step === 1 ? 0 : step;
  const goTo = (index: number) => {
    setStep(index);
    listRef.current?.querySelector<HTMLButtonElement>(`[aria-controls="${id}-${index}"]`)?.focus();
  };
  return (
    <ol
      ref={listRef}
      data-slot="onboarding-steps"
      className={cn("divide-y overflow-hidden rounded-xl border bg-card", className)}
    >
      {STEPS.slice(offset).map(({ title, description, complete, Content }, index) => {
        const stepIndex = index + offset;
        const open = Math.max(offset, visibleStep) === stepIndex;
        return (
          <li key={title}>
            <h3>
              <button
                type="button"
                id={`${id}-trigger-${stepIndex}`}
                aria-expanded={open}
                aria-controls={`${id}-${stepIndex}`}
                onClick={() => setStep(stepIndex)}
                className="group flex w-full items-start gap-4 p-5 text-left outline-none hover:bg-muted/30 focus-visible:bg-muted/50 sm:p-6"
              >
                <StepIndicator index={index} state={stepState(complete(state, storageReady), open)} />
                <span className="min-w-0 flex-1">
                  <span
                    className={cn(
                      "block text-sm font-medium",
                      !open && "text-muted-foreground group-hover:text-foreground",
                    )}
                  >
                    {title}
                  </span>
                  <span className="mt-1 block text-sm leading-6 font-normal text-muted-foreground">{description}</span>
                </span>
                <ChevronDown
                  aria-hidden="true"
                  className={cn("mt-1 size-4 shrink-0 text-muted-foreground", open && "rotate-180")}
                />
              </button>
            </h3>
            <div
              id={`${id}-${stepIndex}`}
              role="region"
              aria-labelledby={`${id}-trigger-${stepIndex}`}
              hidden={!open}
              className="px-5 pb-6 sm:pr-6 sm:pb-7 sm:pl-17"
            >
              <Content state={state} goTo={goTo} storageReady={storageReady} />
            </div>
          </li>
        );
      })}
    </ol>
  );
}
