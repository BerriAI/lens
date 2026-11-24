"use client";

import { useEffect, useEffectEvent } from "react";
import {
  GATEWAY_FRAME_CHANNEL,
  onGatewayFrameCommand,
  type GatewayFrameCommand,
} from "../../host/gatewayFrame";
import type { LensAgents } from "./agents/AgentScoped";
import { ALL_AGENTS_LABEL } from "./agents/AgentPicker";
import { useWorkerConnected } from "./hooks/useWorkerConnected";
import { NAVIGATION_ITEMS } from "./LensTabs";
import type { InvestigationActivity } from "./model/status";
import type { LensList } from "./model/types";
import { LENS_TABS, type LensTab } from "./route";
import { ALL_AGENTS } from "./traces/routing";

interface GatewayFrameNavigationProps {
  readonly activeTab: LensTab;
  readonly agents: LensAgents;
  readonly activity: InvestigationActivity;
  readonly workers: LensList["workers"] | null;
  readonly canViewAlerts: boolean;
  readonly onNavigate: (tab: LensTab) => void;
}

export function GatewayFrameNavigation({
  activeTab,
  agents,
  activity,
  workers,
  canViewAlerts,
  onNavigate,
}: GatewayFrameNavigationProps) {
  const connected = useWorkerConnected(workers);
  const tabs = NAVIGATION_ITEMS.filter(
    ({ value }) => value !== "alerts" || canViewAlerts,
  ).map(({ value, label }) => ({
    value,
    label,
    status:
      value === "findings" && activity !== "idle"
        ? `Analysis is ${activity}`
        : undefined,
  }));
  const visibleTabs = workers
    ? [
        ...tabs,
        {
          value: "settings" as const,
          label: LENS_TABS.settings,
          status: connected ? undefined : "Configure analysis",
        },
      ]
    : tabs;
  const agentOptions = [
    { value: ALL_AGENTS, label: ALL_AGENTS_LABEL },
    ...agents.list.agents.map(({ name }) => ({ value: name, label: name })),
  ];
  const currentAgent = agents.all ? ALL_AGENTS : agents.agent;
  const publish = useEffectEvent(() => {
    window.parent.postMessage(
      {
        channel: GATEWAY_FRAME_CHANNEL,
        type: "navigation",
        navigation: {
          activeTab,
          tabs: visibleTabs,
          currentAgent,
          agents: agentOptions,
          agentLabel: "Agent",
          bookCallLabel: "Book a call",
        },
      },
      window.location.origin,
    );
  });
  const receive = useEffectEvent((command: GatewayFrameCommand) => {
    if (command.type === "tab") {
      const tab = visibleTabs.find(({ value }) => value === command.value);
      if (tab) onNavigate(tab.value);
    } else if (
      command.type === "agent" &&
      agentOptions.some(({ value }) => value === command.value)
    ) {
      agents.select(command.value);
    }
  });
  useEffect(() => onGatewayFrameCommand((command) => receive(command)), []);
  useEffect(() => {
    publish();
  });
  return null;
}
