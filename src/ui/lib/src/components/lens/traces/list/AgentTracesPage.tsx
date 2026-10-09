"use client";

import { useRelativeRange } from "../../../shared/timeRange/useRelativeRange";

import { AgentTracesSection } from "./AgentTracesSection";
import { useTracesLive } from "../api";

export default function AgentTracesPage({
  accessToken,
  isActive = true,
  readOnly = false,
  canMintTracingKey = false,
  canViewFindings = true,
  onSetUpSignals,
  onConnectAgent,
}: {
  accessToken: string;
  isActive?: boolean;
  readOnly?: boolean;
  canMintTracingKey?: boolean;
  canViewFindings?: boolean;
  onSetUpSignals?: () => void;
  onConnectAgent?: () => void;
}) {
  const time = useRelativeRange(useTracesLive());
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <AgentTracesSection
        accessToken={accessToken}
        isActive={isActive}
        range={time.range}
        readOnly={readOnly}
        canMintTracingKey={canMintTracingKey}
        canViewFindings={canViewFindings}
        onSetUpSignals={onSetUpSignals}
        onConnectAgent={onConnectAgent}
        timeControls={time}
      />
    </div>
  );
}
