"use client";

import { useId, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Tabs } from "@base-ui/react/tabs";
import { BookOpen, ChevronRight, Loader2 } from "lucide-react";
import { useLensHost } from "../../host/LensHost";
import AgentTracesPage from "./traces/list/AgentTracesPage";
import { Button } from "../ui/button";
import type { TraceSummary } from "./traces/types";
import { Switch } from "../ui/switch";
import { TabsContent } from "../ui/tabs";
import { LensServicesProvider, useLensAccessToken, useLensApi, useLiveLensServices } from "./data/LensServices";
import { isProxyAdminRole, isProxyAdminTierRole } from "../../utils/roles";
import { InvestigationsView } from "./investigations/InvestigationsView";
import { DatasetsView } from "./datasets/DatasetsView";
import { EvalsView } from "./evals/EvalsView";
import { LensSettings } from "./settings/LensSettings";
import { createLensDemo } from "./data/demo/createLensDemo";
import { lensQueries } from "./data/queries";
import { LensSidebar } from "./LensSidebar";
import { LensTabs } from "./LensTabs";
import { FindingsView } from "./investigations/FindingsView";
import { investigationActivity, listPollInterval } from "./model/status";
import { cn } from "../../lib/cva.config";
import { LENS_TABS, useDialogRoute, useIssueRoute, useLensRoute, type LensDialog, type LensTab } from "./route";
import { LensGettingStarted } from "./onboarding/LensGettingStarted";
import { useLensReadiness, type LensReadiness } from "./hooks/useLensReadiness";
import { OnboardingProvider, type Onboarding } from "./onboarding/OnboardingContext";
import { traceRefOf, useOpenTraceRouting, type TraceRef } from "./traces/routing";
import { useLensAgents } from "./agents/AgentScoped";
import { AgentsView } from "./agents/AgentsView";
import { AgentPicker } from "./agents/AgentPicker";
import { LensHome } from "./onboarding/LensHome";

type WorkspaceProps = {
  accessToken: string;
  userRole: string;
  readOnly: boolean;
};

export function LensWorkspace(props: WorkspaceProps) {
  const { demo } = useLensRoute();
  return demo ? <SampleSession /> : <LiveSession {...props} />;
}

function LiveSession(props: WorkspaceProps) {
  const services = useLiveLensServices(props.accessToken);
  return (
    <LensServicesProvider services={services}>
      <LensContent userRole={props.userRole} readOnly={props.readOnly} />
    </LensServicesProvider>
  );
}

function SampleSession() {
  const [services] = useState(() => createLensDemo());
  return (
    <LensServicesProvider services={services}>
      <LensContent userRole="proxy_admin_viewer" readOnly />
    </LensServicesProvider>
  );
}

function DemoToggle({ demo, onChange }: { demo: boolean; onChange: (demo: boolean) => void }) {
  const id = useId();
  return (
    <div
      className={cn(
        "flex items-center gap-2 whitespace-nowrap text-xs",
        demo ? "font-medium text-info" : "text-muted-foreground",
      )}
    >
      <label htmlFor={id}>Demo data</label>
      <Switch id={id} size="sm" checked={demo} onCheckedChange={onChange} className="data-checked:bg-info" />
    </div>
  );
}

/** The one always-mounted `/lens` observer; every other reader is a plain cache subscriber. */
function useLensOverview(enabled: boolean, settingsOpen: boolean) {
  const api = useLensApi();
  const { data } = useQuery({
    ...lensQueries.list(api),
    enabled,
    refetchInterval: (query) => listPollInterval(query.state.data, settingsOpen, Date.now()),
  });
  return { activity: investigationActivity(data?.lenses ?? []), list: data };
}

const PANEL =
  "flex min-h-0 flex-1 flex-col overflow-y-auto animate-in fade-in-0 duration-300 motion-reduce:animate-none";

function LensContent({ userRole, readOnly }: Omit<WorkspaceProps, "accessToken">) {
  const embedded = useLensHost().surface === "embedded";
  const accessToken = useLensAccessToken();
  const { tab, defaultTab: entryTab, lensId, demo, settingUp, setTab, setDemo, setSetup } = useLensRoute();
  const { dialog, openDialog } = useDialogRoute();
  const { issueKey } = useIssueRoute();
  const { trace, openTrace } = useOpenTraceRouting();
  const agents = useLensAgents(accessToken);
  const canViewInvestigations = isProxyAdminTierRole(userRole);
  const isAdmin = isProxyAdminRole(userRole);
  const canConfigure = canViewInvestigations && !readOnly;
  const defaultTab = embedded && entryTab === "home" ? "traces" : entryTab;
  const activeTab = tab === "settings" && !canConfigure ? defaultTab : tab ?? defaultTab;
  const setupState = useLensReadiness(canViewInvestigations);
  const setupLocation = {
    tab: activeTab,
    requested: settingUp,
    canViewInvestigations,
    trace,
    lensId,
    dialog,
    issueKey,
  };
  const showSetup = !demo && (settingUp || (embedded && needsSetup(setupState, setupLocation)));
  const connectProject = !demo
    ? () => {
        setSetup(false);
        setTab("home");
      }
    : undefined;
  const { activity, list } = useLensOverview(
    canViewInvestigations,
    (canConfigure && activeTab === "settings") || settingUp,
  );
  const workers = canConfigure && list ? list.workers : null;
  const leaveSetup = () => {
    setSetup(false);
  };
  const startSetup = () => {
    setSetup(true);
  };
  const exitSetup = (to: LensTab) => {
    leaveSetup();
    setTab(to);
  };
  const showSettings = () => {
    leaveSetup();
    setTab("settings");
  };
  const startFirstInvestigation = () => {
    leaveSetup();
    setTab("investigations");
    openDialog("new");
  };
  const showSentTrace = (trace: TraceSummary) => {
    leaveSetup();
    setTab("traces");
    openTrace(traceRefOf(trace));
  };
  const toggleDemo = (next: boolean) => {
    leaveSetup();
    setDemo(next);
  };
  const onboarding: Onboarding = {
    readOnly,
    canViewInvestigations,
    canInvestigate: isAdmin,
    canMintTracingKey: isAdmin,
    connect: showSettings,
    create: startFirstInvestigation,
    openTrace: showSentTrace,
  };
  const navigate = (tab: LensTab) => {
    leaveSetup();
    setTab(tab);
  };
  return (
    <OnboardingProvider value={onboarding}>
      <main className="relative flex h-full min-h-0 w-full min-w-0 flex-1 bg-background">
        <Tabs.Root
          value={activeTab}
          orientation={embedded ? "horizontal" : "vertical"}
          onValueChange={(value) => navigate(value as LensTab)}
          className="@container/lens-frame flex min-h-0 min-w-0 flex-1 gap-0"
        >
          {!embedded && <LensSidebar agents={agents} activity={activity} workers={workers} onNavigate={navigate} />}
          <div className="flex min-h-0 min-w-0 flex-1 flex-col">
            <header
              className={cn(
                "flex h-14 shrink-0 items-center justify-between gap-3 border-b bg-card px-4 md:px-6",
                !embedded && "pl-14",
              )}
            >
              <div className="flex min-w-0 items-center gap-2 text-sm">
                {embedded ? (
                  <>
                    <h1 className="font-medium">Lens</h1>
                    {agents.agent && (
                      <>
                        <ChevronRight aria-hidden="true" className="size-3.5 shrink-0 text-muted-foreground/60" />
                        <AgentPicker agent={agents.agent} agents={agents.list.agents} onSelect={agents.select} />
                      </>
                    )}
                  </>
                ) : (
                  <>
                    {activeTab === "traces" && agents.agent && (
                      <>
                        <Button
                          variant="link"
                          size="sm"
                          className="h-auto p-0 text-muted-foreground"
                          onClick={() => navigate("agents")}
                        >
                          Agents
                        </Button>
                        <ChevronRight aria-hidden="true" className="size-3.5 shrink-0 text-muted-foreground/60" />
                        <span className="hidden max-w-64 truncate text-muted-foreground sm:inline">{agents.agent}</span>
                        <ChevronRight
                          aria-hidden="true"
                          className="hidden size-3.5 shrink-0 text-muted-foreground/60 sm:block"
                        />
                      </>
                    )}
                    <h1 className="truncate font-medium">{LENS_TABS[activeTab]}</h1>
                  </>
                )}
              </div>
              <div className="flex shrink-0 items-center gap-3">
                <DemoToggle demo={demo} onChange={toggleDemo} />
                {embedded && (
                  <a
                    href="https://docs.litellm.ai/docs/proxy/lens"
                    target="_blank"
                    rel="noopener noreferrer"
                    aria-label="Documentation"
                    title="Documentation"
                    className="rounded-sm text-muted-foreground hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
                  >
                    <BookOpen aria-hidden="true" className="size-4" />
                  </a>
                )}
              </div>
            </header>
            {embedded && (
              <div className="shrink-0 overflow-x-auto border-b bg-card px-4 md:px-6">
                <LensTabs orientation="horizontal" activity={activity} workers={workers} onNavigate={navigate} />
              </div>
            )}
            <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-card px-3 md:px-5">
              {showSetup ? (
                <TabsContent value={activeTab} keepMounted className={cn(PANEL, "p-3 sm:p-5")}>
                  {setupState.loading ? (
                    <p role="status" className="flex items-center gap-2 py-8 text-sm text-muted-foreground">
                      <Loader2 aria-hidden="true" className="size-4 animate-spin" />
                      Checking Lens setup…
                    </p>
                  ) : (
                    <LensGettingStarted state={setupState} onStart={startSetup} onExit={exitSetup} />
                  )}
                </TabsContent>
              ) : (
                <>
                  <TabsContent value="home" className={PANEL}>
                    <LensHome
                      agents={agents}
                      enabled={!demo}
                      onOpenAgent={agents.select}
                      onOpenAgents={() => navigate("agents")}
                      onSetup={canConfigure ? startSetup : undefined}
                    />
                  </TabsContent>
                  <TabsContent value="agents" className={PANEL}>
                    <AgentsView agents={agents} onOpenAgent={agents.select} onConnectProject={connectProject} />
                  </TabsContent>
                  <TabsContent value="traces" keepMounted className={PANEL}>
                    <AgentTracesPage
                      accessToken={accessToken}
                      isActive={activeTab === "traces"}
                      readOnly={readOnly}
                      canMintTracingKey={isAdmin}
                      canViewFindings={canViewInvestigations}
                      onSetUpSignals={canConfigure ? showSettings : undefined}
                      onConnectAgent={connectProject}
                    />
                  </TabsContent>
                  <TabsContent value="findings" className={PANEL}>
                    {canViewInvestigations ? (
                      <FindingsView readOnly={readOnly || !isAdmin} />
                    ) : (
                      <p className="py-6 text-sm text-muted-foreground">Findings require proxy administrator access.</p>
                    )}
                  </TabsContent>
                  <TabsContent value="investigations" className={PANEL}>
                    {canViewInvestigations ? (
                      <InvestigationsView readOnly={readOnly || !isAdmin} />
                    ) : (
                      <p className="py-6 text-sm text-muted-foreground">
                        Investigations require proxy administrator access. You can still view your traces.
                      </p>
                    )}
                  </TabsContent>
                  <TabsContent value="datasets" className={PANEL}>
                    <DatasetsPanel canView={canViewInvestigations} isAdmin={isAdmin} readOnly={readOnly} />
                  </TabsContent>
                  <TabsContent value="evals" className={PANEL}>
                    {canViewInvestigations ? (
                      <EvalsView />
                    ) : (
                      <p className="py-6 text-sm text-muted-foreground">Evals require proxy administrator access.</p>
                    )}
                  </TabsContent>
                </>
              )}
              {workers && list && (
                <TabsContent value="settings" keepMounted className={cn(PANEL, "p-3 sm:p-5")}>
                  <LensSettings
                    list={list}
                    workerReadyAction={
                      list.lenses.length === 0 ? (
                        <Button className="w-full" onClick={startFirstInvestigation}>
                          New investigation
                        </Button>
                      ) : undefined
                    }
                    onOpenTraces={() => setTab("traces")}
                  />
                </TabsContent>
              )}
            </div>
          </div>
        </Tabs.Root>
      </main>
    </OnboardingProvider>
  );
}

function DatasetsPanel({ canView, isAdmin, readOnly }: { canView: boolean; isAdmin: boolean; readOnly: boolean }) {
  if (!canView)
    return <p className="py-6 text-sm text-muted-foreground">Datasets require proxy administrator access.</p>;
  return <DatasetsView readOnly={readOnly || !isAdmin} />;
}

function needsSetup(
  state: LensReadiness,
  location: {
    tab: LensTab;
    requested: boolean;
    canViewInvestigations: boolean;
    trace: TraceRef | null;
    lensId: string | null;
    dialog: LensDialog | null;
    issueKey: string | null;
  },
) {
  if (
    location.tab === "settings" ||
    location.tab === "home" ||
    location.tab === "datasets" ||
    location.tab === "evals" ||
    location.tab === "agents"
  )
    return false;
  if (location.requested) return true;
  const selected = location.tab === "traces" ? location.trace : location.lensId || location.dialog || location.issueKey;
  if (!state.missingTraces || selected) return false;
  if (location.tab === "traces") return true;
  return location.canViewInvestigations && !state.hasInvestigations && !state.hasRecordedActivity;
}
