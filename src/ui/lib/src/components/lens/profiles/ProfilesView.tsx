"use client";

import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { ChartNoAxesColumn, CircleAlert, List, RefreshCw } from "lucide-react";
import { Button } from "../../ui/button";
import { LoadingState } from "../../shared/LoadingState";
import { StateMessage } from "../../shared/StateMessage";
import { TimeRangeControls } from "../../shared/timeRange/TimeRangeControls";
import { timeWindow } from "../../shared/timeRange/timeRange";
import { useRelativeRange } from "../../shared/timeRange/useRelativeRange";
import { useZoomRouting } from "../../shared/timeRange/routing";
import type { LensAgents } from "../agents/AgentScoped";
import { useProfilesApi } from "../data/LensServices";
import { useTracesLive } from "../traces/api";
import { LensPageHeader } from "../ui/LensPageHeader";
import { useLensRoute, useProfileRoute } from "../route";
import { ProfileDetailSheet } from "./ProfileDetailSheet";
import { ProfileHeatmap } from "./ProfileHeatmap";
import { ProfileBreakdown } from "./ProfileBreakdown";
import { ProfileMap } from "./ProfileMap";
import { SegmentedControl } from "./SegmentedControl";
import { TaxonomyPanel } from "./TaxonomyPanel";
import { formatCount, formatUsd, profileColors, selectedProfileId } from "./profilesData";
import type { ProfilesOverview } from "./schemas";

const VIEW_OPTIONS = [
  { value: "breakdown", label: "Breakdown" },
  { value: "similarity", label: "Similarity map" },
] as const;

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
    <div className="space-y-2">
      <div className="grid gap-3 sm:grid-cols-3">
        {cards.map((card) => (
          <div key={card.label} className="rounded-xl border bg-card p-4">
            <p className="text-xs text-muted-foreground">{card.label}</p>
            <p className="text-2xl font-semibold tracking-tight tabular-nums">{card.value}</p>
            <p className="text-xs text-muted-foreground">{card.note}</p>
          </div>
        ))}
      </div>
      <p className="text-xs text-muted-foreground">
        {!live && "Demo data. "}
        {overview.taxonomy?.sample && "Sample data. "}
        {coverage.truncated &&
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
}: {
  agents: LensAgents;
  canInvestigate?: boolean;
}) {
  const api = useProfilesApi();
  const time = useRelativeRange(false);
  const live = useTracesLive();
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
  const matrix = useQuery({
    queryKey: ["lens", "profile-matrix", api.scope, window, agent],
    queryFn: ({ signal }) => api.matrix(window, agent, signal),
    enabled: !!agent,
    staleTime: 30_000,
  });
  const profiles = overview.data?.profiles ?? [];
  const colors = useMemo(() => profileColors(profiles), [profiles]);
  const selected = selectedProfileId(profiles, profile.profile);
  const fetching = overview.isFetching || matrix.isFetching;
  return (
    <section aria-label="Usage profiles" className="flex min-h-0 flex-1 flex-col">
      <LensPageHeader
        page="profiles"
        title="Usage profiles"
        description="Understand the different tasks your agents are used for"
        actions={
          <>
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
              onClick={() => (zoom ? void Promise.all([overview.refetch(), matrix.refetch()]) : setRefreshAt(Date.now()))}
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
        {overview.data && overview.data.totals.runs === 0 && (
          <StateMessage
            role="status"
            icon={<ChartNoAxesColumn className="size-5" />}
            title="No runs in this time range"
            description="Choose another agent or a wider time range to see its task breakdown."
          />
        )}
        {overview.data && overview.data.totals.runs > 0 && (
          <>
            <Totals overview={overview.data} agent={agent} live={live} />
            <div className="rounded-xl border bg-card p-5">
              <div className="mb-4 flex flex-wrap items-center justify-between gap-2">
                <h3 className="text-base font-medium">Profiles</h3>
                <SegmentedControl label="Profile view" options={VIEW_OPTIONS} value={profile.view} onChange={profile.setView} />
              </div>
              {profile.view === "breakdown" ? (
                <ProfileBreakdown profiles={profiles} colors={colors} selected={selected} onSelect={profile.selectProfile} />
              ) : (
                <ProfileMap overview={overview.data} colors={colors} selected={selected} onSelect={profile.selectProfile} />
              )}
            </div>
            <div className="rounded-xl border bg-card p-5">
              <h3 className="mb-3 text-base font-medium">Profiles by model deployment</h3>
              {matrix.error && <p role="alert" className="text-sm text-destructive">{matrix.error.message}</p>}
              {matrix.isPending && <p className="text-sm text-muted-foreground">Loading model figures…</p>}
              {matrix.data && <ProfileHeatmap matrix={matrix.data} colors={colors} selected={selected} onSelect={profile.selectProfile} />}
            </div>
          </>
        )}
        {!!agent && <TaxonomyPanel agent={agent} window={window} />}
      </div>
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
