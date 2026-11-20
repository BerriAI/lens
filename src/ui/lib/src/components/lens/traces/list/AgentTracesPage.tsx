"use client";

import { useRelativeRange } from "../../../shared/timeRange/useRelativeRange";

import { AgentTracesSection } from "./AgentTracesSection";
import { useTracesApi, useTracesLive } from "../api";

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
  const filters = useTracesApi(accessToken).filters;
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {(filters?.team_id || filters?.api_key_hash) && (
        <div
          className="flex flex-wrap gap-x-4 gap-y-1 border-b px-4 py-2 text-xs text-muted-foreground"
          aria-label="Trace scope"
        >
          {filters.team_id && <span>Team: {filters.team_id}</span>}
          {filters.api_key_hash && <span>Key: {filters.api_key_hash.slice(0, 12)}…</span>}
        </div>
      )}
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
