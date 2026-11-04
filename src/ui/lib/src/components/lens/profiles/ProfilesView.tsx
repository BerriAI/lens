"use client";

import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { ChartNoAxesColumn, RefreshCw } from "lucide-react";
import { parseAsString, useQueryState } from "nuqs";
import { Button } from "../../ui/button";
import { TimeRangeControls } from "../../shared/timeRange/TimeRangeControls";
import { timeWindow } from "../../shared/timeRange/timeRange";
import { useRelativeRange } from "../../shared/timeRange/useRelativeRange";
import { useZoomRouting } from "../../shared/timeRange/routing";
import type { LensAgents } from "../agents/AgentScoped";
import { useProfilesApi } from "../data/LensServices";
import { useTracesLive } from "../traces/api";
import { LensPageHeader } from "../ui/LensPageHeader";
import { useProfileRoute } from "../route";
import { summarizeUsage, type UsageCategory } from "./usageProfile";

const COLORS = ["#6366f1", "#0ea5e9", "#14b8a6", "#f59e0b", "#8b5cf6"];
const percent = (count: number, total: number) =>
  `${((count / total) * 100).toLocaleString(undefined, { maximumFractionDigits: 1 })}%`;

function UsageGraph({
  categories,
  total,
  selected,
  onSelect,
}: {
  categories: UsageCategory[];
  total: number;
  selected: string | null;
  onSelect: (category: string) => void;
}) {
  const color = (category: UsageCategory, index: number) =>
    category.category === "Unclassified" ? "#94a3b8" : COLORS[index % COLORS.length];
  const gradient = categories
    .map((category, index) => {
      const start = (categories.slice(0, index).reduce((sum, entry) => sum + entry.runs, 0) / total) * 100;
      return `${color(category, index)} ${start}% ${start + (category.runs / total) * 100}%`;
    })
    .join(", ");
  return (
    <figure aria-label="Agent usage by task category" className="space-y-7">
      <div className="grid items-center gap-8 md:grid-cols-[minmax(0,1fr)_minmax(0,1.2fr)]">
        <div
          role="img"
          aria-label={`Usage pie chart: ${categories.map((category) => `${category.category} ${percent(category.runs, total)}`).join(", ")}`}
          className="mx-auto aspect-square w-full max-w-72 rounded-full shadow-sm"
          style={{ background: `conic-gradient(${gradient})` }}
        />
        <div className="space-y-2">
          {categories.map((category, index) => (
            <button
              key={category.category}
              type="button"
              onClick={() => onSelect(category.category)}
              aria-pressed={selected === category.category}
              className="flex w-full items-center justify-between gap-4 rounded-lg border border-transparent px-3 py-3 text-left text-sm hover:bg-muted/60 aria-pressed:border-primary/25 aria-pressed:bg-muted/60"
            >
              <span className="flex min-w-0 items-center gap-2.5 font-medium">
                <span
                  aria-hidden="true"
                  className="size-2.5 shrink-0 rounded-full"
                  style={{ background: color(category, index) }}
                />
                <span className="truncate">{category.category}</span>
              </span>
              <span className="shrink-0 tabular-nums">
                {category.runs.toLocaleString()}{" "}
                <span className="text-muted-foreground">runs · {percent(category.runs, total)}</span>
              </span>
              <span
                role="meter"
                aria-label={category.category}
                aria-valuenow={category.runs}
                aria-valuemin={0}
                aria-valuemax={total}
                aria-valuetext={`${category.runs} of ${total} runs, ${percent(category.runs, total)}`}
                className="sr-only"
              />
            </button>
          ))}
        </div>
      </div>
      <figcaption className="border-t pt-4 text-xs leading-5 text-muted-foreground">
        Each run is counted once across the full time range. Categories come from recorded task_category or
        scenario metadata. Runs without either are Unclassified.
      </figcaption>
    </figure>
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
  const [agent, setAgent] = useQueryState("agent", parseAsString.withDefault(""));
  const anchorMs = Math.max(time.range.anchorMs ?? 0, refreshAt);
  const relativeWindow = useMemo(
    () => timeWindow({ hours: time.range.hours, anchorMs }, anchorMs),
    [time.range.hours, anchorMs],
  );
  const window = zoom ?? relativeWindow;
  const query = useQuery({
    queryKey: ["lens", "usage-profile", api.scope, window, agent],
    queryFn: ({ signal }) => api.usage(window, agent, signal),
    staleTime: 30_000,
  });
  const summary = summarizeUsage(query.data ?? []);
  return (
    <section aria-label="Usage profiles" className="flex min-h-0 flex-1 flex-col">
      <LensPageHeader
        page="profiles"
        title="Usage profiles"
        description="Understand the different tasks your agents are used for"
        actions={
          <>
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
              disabled={query.isFetching}
              onClick={() => (zoom ? void query.refetch() : setRefreshAt(Date.now()))}
            >
              <RefreshCw aria-hidden="true" className={`size-4 ${query.isFetching ? "animate-spin" : ""}`} />
            </Button>
          </>
        }
      />
      <div className="lens-page-body flex flex-col gap-6">
        <label className="grid max-w-sm gap-2 text-sm font-medium">
          Agent
          <select
            aria-label="Profile agent"
            value={agent}
            onChange={(event) => void setAgent(event.target.value)}
            className="h-9 rounded-md border bg-background px-3 text-sm"
          >
            <option value="">All agents</option>
            {agent && !agents.list.agents.some((item) => item.name === agent) && (
              <option value={agent}>{agent}</option>
            )}
            {agents.list.agents.map((item) => (
              <option key={item.name} value={item.name}>
                {item.name}
              </option>
            ))}
          </select>
        </label>
        {query.isPending && (
          <p role="status" className="py-12 text-center text-sm text-muted-foreground">
            Loading usage profiles…
          </p>
        )}
        {query.error && (
          <div role="alert" className="rounded-xl border p-6 text-sm text-destructive">
            Could not load usage profiles. {query.error.message}
          </div>
        )}
        {!query.isPending && !query.error && summary.total === 0 && (
          <div className="flex flex-col items-center gap-3 rounded-xl border border-dashed px-6 py-16 text-center">
            <ChartNoAxesColumn aria-hidden="true" className="size-8 text-muted-foreground" />
            <h3 className="font-medium">No runs in this time range</h3>
            <p className="max-w-sm text-sm text-muted-foreground">
              Choose another agent or a wider time range to see its task breakdown.
            </p>
          </div>
        )}
        {!query.error && summary.total > 0 && (
          <div className="max-w-4xl rounded-xl border bg-card p-5 md:p-8">
            <div className="mb-8 flex flex-wrap items-start justify-between gap-4">
              <div>
                <h3 className="text-base font-medium">Task breakdown</h3>
                <p className="mt-1 text-sm text-muted-foreground">
                  {agent || "All agents"}
                  {!live && " · Demo data"}
                </p>
              </div>
              <div className="text-right">
                <p className="text-3xl font-semibold tracking-tight tabular-nums">
                  {summary.total.toLocaleString()}
                </p>
                <p className="text-xs text-muted-foreground">runs in selected range</p>
              </div>
            </div>
            <UsageGraph
              categories={summary.categories}
              total={summary.total}
              selected={profile.profile}
              onSelect={profile.selectProfile}
            />
            {canInvestigate && (
              <div className="mt-5 flex flex-wrap items-center justify-between gap-3 border-t pt-5">
                <p className="text-sm text-muted-foreground">
                {zoom
                  ? "Choose a relative time range to investigate this profile. Investigations review recent runs."
                  : profile.profile
                    ? `Review ${profile.profile} runs with Lens`
                    : "Choose a task category to investigate its runs"}
                </p>
                <Button
                  disabled={
                  !!zoom ||
                    !profile.profile ||
                    !summary.categories.some((category) => category.category === profile.profile)
                  }
                  onClick={() =>
                    profile.profile && profile.investigate(profile.profile, agent, time.range.hours)
                  }
                >
                  Investigate profile
                </Button>
              </div>
            )}
            {summary.classified === 0 && (
              <p className="mt-4 text-sm text-muted-foreground">
                Record task_category or scenario on your traces to see Coding, Workflows, or your own task
                categories here.
              </p>
            )}
          </div>
        )}
      </div>
    </section>
  );
}
