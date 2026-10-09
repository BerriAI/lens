"use client";

import { ArrowRight, Bot, Search } from "lucide-react";

import { useNow } from "../../../hooks/useNow";
import { formatActivityTimestamp } from "../../../utils/activityTimestamp";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "../../ui/table";
import { agoLabel } from "../model/format";
import { useAgentSearchRoute } from "../route";
import { traceFramework } from "../traces/ui/TraceFramework";
import { AgentMark, matchesAgent } from "./AgentPicker";
import type { LensAgents } from "./AgentScoped";
import { AGENT_WINDOW_DAYS } from "./useAgents";

export function AgentsView({ agents, onOpenAgent }: { agents: LensAgents; onOpenAgent: (agent: string) => void }) {
  const [search, setSearch] = useAgentSearchRoute();
  const now = useNow(30_000);
  const { agents: all, isLoading, error } = agents.list;
  const shown = all.filter((agent) => matchesAgent(agent, search));

  return (
    <section aria-label="Agents directory" className="flex flex-col gap-5 p-4 sm:p-6">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div className="space-y-1">
          <h2 className="flex items-center gap-2 text-lg font-semibold tracking-tight">
            Agents
            {!isLoading && !error && (
              <span className="rounded-md bg-muted px-1.5 py-0.5 text-xs font-medium text-muted-foreground tabular-nums">
                {all.length.toLocaleString()}
              </span>
            )}
          </h2>
          <p className="text-sm text-muted-foreground">Choose an agent to explore its traces and activity</p>
        </div>
        <span className="text-xs text-muted-foreground">Last {AGENT_WINDOW_DAYS} days</span>
      </div>

      <div className="relative w-full sm:max-w-80">
        <Search
          aria-hidden
          className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground"
        />
        <Input
          aria-label="Search agents"
          placeholder="Search agents…"
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          className="h-9 bg-background pl-9 shadow-none"
        />
      </div>

      {isLoading && (
        <p role="status" className="py-16 text-center text-sm text-muted-foreground">
          Loading agents…
        </p>
      )}
      {!isLoading && error && (
        <p role="alert" className="rounded-lg border p-6 text-sm text-destructive">
          Could not load agents. Refresh the page to try again.
        </p>
      )}
      {!isLoading && !error && all.length === 0 && (
        <div className="flex flex-col items-center gap-3 rounded-xl border border-dashed px-6 py-16 text-center">
          <span className="flex size-10 items-center justify-center rounded-lg border bg-muted/40">
            <Bot aria-hidden className="size-5 text-muted-foreground" />
          </span>
          <div className="space-y-1">
            <h3 className="text-sm font-medium">No agents yet</h3>
            <p className="max-w-sm text-sm text-muted-foreground">Agents appear here when Lens receives their traces</p>
          </div>
        </div>
      )}
      {!isLoading && !error && all.length > 0 && (
        <div className="overflow-hidden rounded-xl border bg-card">
          <Table aria-label="Agents">
            <TableHeader>
              <TableRow className="hover:bg-transparent">
                <TableHead className="px-4 text-xs font-normal text-muted-foreground">Agent</TableHead>
                <TableHead className="px-4 text-right text-xs font-normal text-muted-foreground">Runs</TableHead>
                <TableHead className="px-4 text-right text-xs font-normal text-muted-foreground">Errors</TableHead>
                <TableHead className="px-4 text-right text-xs font-normal text-muted-foreground">Last active</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {shown.map((agent) => {
                const framework = traceFramework({
                  frameworks: [...agent.frameworks],
                });
                return (
                  <TableRow key={agent.name}>
                    <TableCell className="px-4 py-3">
                      <Button
                        variant="ghost"
                        aria-label={`Open ${agent.name}`}
                        onClick={() => onOpenAgent(agent.name)}
                        className="group -ml-2 h-auto max-w-full justify-start gap-3 px-2 py-1.5 text-left"
                      >
                        <span className="flex size-9 shrink-0 items-center justify-center rounded-lg border bg-background">
                          <AgentMark agent={agent} />
                        </span>
                        <span className="flex min-w-0 flex-col gap-0.5">
                          <span className="max-w-64 truncate text-sm font-medium">{agent.name}</span>
                          <span className="text-xs font-normal text-muted-foreground">
                            {framework?.label ?? "Agent"}
                          </span>
                        </span>
                        <ArrowRight
                          aria-hidden
                          className="ml-3 size-3.5 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100"
                        />
                      </Button>
                    </TableCell>
                    <TableCell className="px-4 text-right tabular-nums">{agent.runs.toLocaleString()}</TableCell>
                    <TableCell className="px-4 text-right tabular-nums">
                      <span className={agent.failed_runs > 0 ? "text-destructive" : "text-muted-foreground"}>
                        {agent.failed_runs.toLocaleString()}
                      </span>
                    </TableCell>
                    <TableCell className="px-4 text-right text-xs text-muted-foreground">
                      <time dateTime={agent.last_seen} title={formatActivityTimestamp(agent.last_seen)}>
                        {agoLabel(Date.parse(agent.last_seen), now)}
                      </time>
                    </TableCell>
                  </TableRow>
                );
              })}
              {shown.length === 0 && (
                <TableRow>
                  <TableCell colSpan={4} className="py-12 text-center text-sm text-muted-foreground">
                    No agents match “{search}”
                  </TableCell>
                </TableRow>
              )}
            </TableBody>
          </Table>
        </div>
      )}
    </section>
  );
}
