"use client";

import { type ReactNode } from "react";
import { Activity, ArrowUpRight, Cpu } from "lucide-react";
import { Button } from "../../ui/button";
import { StatusDot } from "../../shared/StatusDot";
import { SignalSettings } from "./signals/SignalSettings";
import { SettingsCard, SettingsSection } from "./SettingsSection";
import type { LensList } from "../model/types";
import { STANDALONE_DOCS_URL, useLensHost } from "../../../host/LensHost";
import { DeploymentAnalysis } from "./worker/DeploymentAnalysis";
import { LensPageHeader } from "../ui/LensPageHeader";

const TRACING_DOCS = "https://docs.litellm.ai/docs/proxy/lens";

function TracingSection({
  enabled,
  onConnectProject,
}: {
  enabled: boolean;
  onConnectProject?: () => void;
}) {
  const standalone = useLensHost().surface === "standalone";
  return (
    <SettingsSection
      heading="Tracing"
      icon={<Activity aria-hidden="true" className="size-4" />}
      description="Where your agents send runs so Lens can read them."
    >
      <SettingsCard className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-3 text-sm">
          <span
            role="status"
            className="inline-flex items-center gap-2 font-mono text-xs"
          >
            <StatusDot state={enabled ? "ok" : "off"} />
            {enabled ? "Tracing enabled" : "Tracing is not enabled"}
          </span>
        </div>
        <div className="flex items-center gap-3">
          <a
            href={standalone ? STANDALONE_DOCS_URL : TRACING_DOCS}
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex items-center gap-1 text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
          >
            Docs
            <ArrowUpRight aria-hidden="true" className="size-3" />
          </a>
          <Button
            variant="outline"
            size="sm"
            onClick={onConnectProject}
            disabled={!onConnectProject}
          >
            Connect project
          </Button>
        </div>
      </SettingsCard>
    </SettingsSection>
  );
}

export function LensSettings({
  list,
  workerReadyAction,
  onConnectProject,
}: {
  list: LensList;
  workerReadyAction?: ReactNode;
  onConnectProject?: () => void;
}) {
  return (
    <div aria-label="Settings" role="region" className="flex w-full flex-col">
      <LensPageHeader
        section="07 / CONFIGURATION"
        title="Settings"
        description="Wire up your agents. Tune what Lens watches."
      />
      <div className="mx-auto w-full max-w-6xl divide-y divide-border px-4 py-5 sm:px-6">
        <TracingSection
          enabled={list.tracing_enabled}
          onConnectProject={onConnectProject}
        />
        <SignalSettings />
        <SettingsSection
          heading="Analysis"
          icon={<Cpu aria-hidden="true" className="size-4" />}
          description="Lens runs investigations using your configured models."
        >
          <DeploymentAnalysis
            workers={list.workers}
            readyAction={workerReadyAction}
          />
        </SettingsSection>
      </div>
    </div>
  );
}
