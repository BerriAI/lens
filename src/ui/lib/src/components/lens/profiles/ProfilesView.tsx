"use client";

import { useMemo, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ChartNoAxesColumn, CircleAlert, List, RefreshCw, Sparkles } from "lucide-react";
import { Button } from "../../ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../../ui/tabs";
import { LoadingState } from "../../shared/LoadingState";
import { SearchSelect } from "../../shared/SearchSelect";
import { StateMessage } from "../../shared/StateMessage";
import { TimeRangeControls } from "../../shared/timeRange/TimeRangeControls";
import { timeWindow } from "../../shared/timeRange/timeRange";
import { useRelativeRange } from "../../shared/timeRange/useRelativeRange";
import { useZoomRouting } from "../../shared/timeRange/routing";
import type { LensAgents } from "../agents/AgentScoped";
import { useLensApi, useProfilesApi } from "../data/LensServices";
import { lensQueries } from "../data/queries";
import { useTracesLive } from "../traces/api";
import { LensPageHeader } from "../ui/LensPageHeader";
import { PROFILE_VIEWS, useLensRoute, useProfileRoute } from "../route";
import { ProfileDetailSheet } from "./ProfileDetailSheet";
import { ProfileHeatmap } from "./ProfileHeatmap";
import { DiscoverySettingsSheet } from "./DiscoverySettingsSheet";
import { ProfileBreakdown } from "./ProfileBreakdown";
import { ProfileMap } from "./ProfileMap";
import { formatCount, formatUsd, profileColors, selectedProfileId } from "./profilesData";
import type { ProfilesOverview } from "./schemas";

function Totals({ overview, agent, live }: { overview: ProfilesOverview; agent: string; live: boolean }) {
  const { totals, coverage } = overview;
  const cards = [
    { label: "Runs", value: formatCount(totals.runs), note: coverage.truncated ? `at least, first ${formatCount(coverage.runs_listed)} listed` : "each run counted once" },
    {
      label: `Requests by ${agent}`,
      value: formatCount(totals.requests),
      note: `${formatCount(totals.matched_requests)} matched to gateway records`,
    },
    {
      label: "Known spend",
      value: formatUsd(totals.spend_usd) ?? "No data",
      note: `cost known for ${formatCount(totals.cost_observations)} of ${formatCount(totals.requests)} requests`,
    },
  ];
  return (
    <div className="flex flex-col gap-2">
      <dl className="flex flex-wrap gap-x-8 gap-y-3">
        {cards.map((card) => (
          <div key={card.label} className="min-w-0">
            <dt className="text-xs text-muted-foreground">{card.label}</dt>
            <dd className="text-lg font-semibold tabular-nums">{card.value}</dd>
            <dd className="text-xs text-muted-foreground">{card.note}</dd>
          </div>
        ))}
      </dl>
      <p className="text-xs text-muted-foreground">
        {!live && "Demo data. "}
        {coverage.runs_resolved < coverage.runs_listed &&
          `Request, cost and latency figures cover ${formatCount(coverage.runs_resolved)} of ${formatCount(coverage.runs_listed)} listed runs. Narrow the time range for complete figures. `}
        {totals.other_agent_requests + totals.shared_requests > 0 &&
          `${formatCount(totals.other_agent_requests)} requests by other agents and ${formatCount(totals.shared_requests)} requests shared between agents in these runs are excluded.`}
      </p>
    </div>
  );
}

export function ProfilesView({
  agents,
  canInvestigate = true,
  canViewSignals = false,
}: {
  agents: LensAgents;
  canInvestigate?: boolean;
  canViewSignals?: boolean;
}) {
  const api = useProfilesApi();
  const lensApi = useLensApi();
  const time = useRelativeRange(false);
  const live = useTracesLive();
  const queryClient = useQueryClient();
  const [refreshAt, setRefreshAt] = useState(Date.now);
  const [zoom, setZoom] = useZoomRouting();
  const profile = useProfileRoute();
  const agent = agents.agent ?? "";
  const route = useLensRoute();
  const anchorMs = Math.max(time.range.anchorMs ?? 0, refreshAt);
  const relativeWindow = useMemo(
    () => timeWindow({ hours: time.range.hours, anchorMs }, anchorMs),
    [time.range.hours, anchorMs],
  );
  const window = zoom ?? relativeWindow;
  const overview = useQuery({
    queryKey: ["lens", "profile-overview", api.scope, window, agent],
    queryFn: ({ signal }) => api.overview(window, agent, signal),
    enabled: !!agent,
    staleTime: 30_000,
  });
  const showSignalPicker = canViewSignals && live && profile.view === "breakdown" && !!agent;
  const signalConfig = useQuery({ ...lensQueries.signalConfig(lensApi), enabled: showSignalPicker });
  const signals = showSignalPicker && signalConfig.data?.model ? signalConfig.data.signals : [];
  const signalId = signals.find(({ id }) => id === profile.signalId)?.id;
  const signalCounts = useQuery({
    queryKey: ["lens", "profile-signal-counts", api.scope, window, agent, signalId, signalConfig.data],
    queryFn: ({ signal }) => api.signalCounts(window, agent, signalId ?? "", signal),
    enabled: !!signalId && !!overview.data?.profiles.length,
  });
  const matrix = useQuery({
    queryKey: ["lens", "profile-matrix", api.scope, window, agent],
    queryFn: ({ signal }) => api.matrix(window, agent, signal),
    enabled: !!agent,
    staleTime: 30_000,
  });
  const taxonomy = useQuery({
    queryKey: ["lens", "profile-taxonomy", api.scope, agent],
    queryFn: ({ signal }) => api.taxonomy(agent, signal),
    enabled: !!agent,
  });
  const [configuring, setConfiguring] = useState(false);
  const shown = overview.error ? undefined : overview.data;
  const heatmap = matrix.error ? undefined : matrix.data;
  const profiles = shown?.profiles ?? [];
  const colors = useMemo(() => profileColors(profiles), [profiles]);
  const selected = selectedProfileId(profiles, profile.profile);
  const fetching = overview.isFetching || matrix.isFetching || signalCounts.isFetching;
  return (
    <section aria-label="Usage profiles" className="flex min-h-0 flex-1 flex-col">
      <LensPageHeader
        page="profiles"
        title="Usage profiles"
        description="Understand the different tasks your agents are used for"
        actions={
          <>
            {taxonomy.data && (taxonomy.data.can_write || taxonomy.data.active || taxonomy.data.taxonomy) && (
              <Button variant="outline" size="sm" onClick={() => setConfiguring(true)}>
                <Sparkles className="size-3.5" aria-hidden="true" /> Task discovery
              </Button>
            )}
            <Button variant="outline" size="sm" onClick={() => route.setTab("traces")}>
              <List className="size-3.5" aria-hidden="true" /> View traces
            </Button>
            <TimeRangeControls
              range={{ ...time.range, anchorMs }}
              zoom={zoom}
              onHoursChange={(hours) => {
                setZoom(null);
                time.setHours(hours);
              }}
              showLive={false}
              onLiveChange={time.setLive}
            />
            <Button
              variant="outline"
              size="icon-sm"
              aria-label="Refresh usage profiles"
              disabled={fetching}
              onClick={() =>
                zoom
                  ? void Promise.all([
                      overview.refetch(),
                      matrix.refetch(),
                      ...(signalId ? [signalCounts.refetch()] : []),
                      queryClient.invalidateQueries({ queryKey: ["lens", "profile-detail", api.scope, window, agent] }),
                    ])
                  : setRefreshAt(Date.now())
              }
            >
              <RefreshCw aria-hidden="true" className={`size-4 ${fetching ? "animate-spin" : ""}`} />
            </Button>
          </>
        }
      />
      <div className="lens-page-body flex flex-col gap-6">
        {((!!agent && overview.isPending) || agents.list.isLoading) && (
          <LoadingState title="Loading usage profiles…" description="Fetching profiles for the selected agent and time range." />
        )}
        {!agent && !agents.list.isLoading && (
          <StateMessage
            role="status"
            icon={<ChartNoAxesColumn className="size-5" />}
            title="Select an agent to explore its tasks"
            description="Choose an agent from the navigation to see its task breakdown."
          />
        )}
        {overview.error && (
          <StateMessage
            role="alert"
            tone="destructive"
            icon={<CircleAlert className="size-5" />}
            title="Could not load usage profiles"
            description={overview.error.message}
          />
        )}
        {shown && shown.totals.runs === 0 && (
          <StateMessage
            role="status"
            icon={<ChartNoAxesColumn className="size-5" />}
            title="No runs in this time range"
            description="Choose another agent or a wider time range to see its task breakdown."
          />
        )}
        {shown && shown.totals.runs > 0 && (
          <>
            <Totals overview={shown} agent={agent} live={live} />
            <Tabs
              value={profile.view}
              onValueChange={(value) => {
                const view = PROFILE_VIEWS.find((candidate) => candidate === value);
                if (view) profile.setView(view);
              }}
              className="lens-panel gap-4 p-4"
            >
              <div className="flex flex-wrap items-center justify-between gap-2">
                <h3 className="lens-section-label">Profiles</h3>
                <TabsList aria-label="Profile view">
                  <TabsTrigger value="breakdown">Breakdown</TabsTrigger>
                  <TabsTrigger value="similarity">Similarity map</TabsTrigger>
                </TabsList>
              </div>
              <TabsContent value="breakdown">
                {signals.length > 0 && (
                  <div className="mb-3 flex flex-wrap items-center gap-2">
                    <div className="w-64 max-w-full">
                      <SearchSelect
                        aria-label="Signal"
                        placeholder="Select a Signal"
                        options={signals.map(({ id, name }) => ({ value: id, label: name }))}
                        value={signalId}
                        onValueChange={profile.selectSignal}
                      />
                    </div>
                    {signalId && signalCounts.isFetching && (
                      <span role="status" className="text-xs text-muted-foreground">Loading flagged runs…</span>
                    )}
                    {signalId && signalCounts.isError && (
                      <span role="alert" className="text-xs text-destructive">Could not load flagged runs</span>
                    )}
                  </div>
                )}
                <ProfileBreakdown
                  profiles={profiles}
                  colors={colors}
                  selected={selected}
                  onSelect={profile.selectProfile}
                  signalCounts={!signalCounts.isError ? signalCounts.data?.by_profile : undefined}
                />
              </TabsContent>
              <TabsContent value="similarity">
                <ProfileMap overview={shown} colors={colors} selected={selected} onOpenDetails={profile.selectProfile} />
              </TabsContent>
            </Tabs>
            <section aria-label="Profiles by model deployment" className="lens-panel flex flex-col gap-3 p-4">
              <h3 className="lens-section-label">Profiles by model deployment</h3>
              {matrix.error && <p role="alert" className="text-xs text-destructive">{matrix.error.message}</p>}
              {matrix.isPending && <p className="text-xs text-muted-foreground">Loading model figures…</p>}
              {heatmap && <ProfileHeatmap matrix={heatmap} colors={colors} selected={selected} onSelect={profile.selectProfile} />}
            </section>
          </>
        )}
      </div>
      {!!agent && (
        <DiscoverySettingsSheet agent={agent} window={window} open={configuring} onOpenChange={setConfiguring} />
      )}
      <ProfileDetailSheet
        agent={agent}
        window={window}
        profileId={selected}
        canInvestigate={canInvestigate}
        investigateDisabled={!!zoom}
        onInvestigate={(profileId) => profile.investigate(profileId, agent, time.range.hours)}
        onClose={() => profile.selectProfile(null)}
      />
    </section>
  );
}
