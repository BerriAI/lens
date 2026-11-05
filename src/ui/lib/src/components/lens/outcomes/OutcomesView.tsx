"use client";

import { useMemo, useState } from "react";
import {
  ArrowRight,
  ChevronRight,
  CircleCheck,
  Download,
  GitMerge,
  GitPullRequest,
  Link2,
} from "lucide-react";
import { Button } from "../../ui/button";
import { Badge } from "../../ui/badge";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../ui/select";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "../../ui/table";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "../../ui/sheet";
import { StateMessage } from "../../shared/StateMessage";
import { LensPageHeader } from "../ui/LensPageHeader";
import { useLensRoute, useOutcomesRoute } from "../route";
import {
  createOutcomeDemo,
  dailyAgentOutcomeTrend,
  agentOutcomes,
  type OutcomeMetric,
  filterOutcomeSessions,
  repositoryOutcomes,
  summarizeOutcomes,
  type OutcomeSession,
} from "./outcomesModel";
import { OutcomesTrend, OUTCOME_AGENT_COLORS, OUTCOME_MEASURES, outcomeValue } from "./OutcomesTrend";
import { Tabs, TabsList, TabsTrigger } from "../../ui/tabs";
import { outcomeTraceId } from "./outcomeTrace";

const money = (value: number) => value.toLocaleString("en-US", { style: "currency", currency: "USD" });
const percentage = (value: number) => `${Math.round(value * 100)}%`;
const date = (value: string) =>
  new Date(value).toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
  });
const STATES = [
  { value: "all", label: "All outcomes" },
  { value: "shipped", label: "Shipped work" },
  { value: "in_progress", label: "In progress" },
  { value: "unlinked", label: "Unlinked sessions" },
] as const;
const PERIODS = [
  { value: "7", label: "Last 7 days" },
  { value: "30", label: "Last 30 days" },
  { value: "90", label: "Last 90 days" },
] as const;

function OutcomeBadges({ session }: { session: OutcomeSession }) {
  const pr = session.pullRequest;
  const issue = session.issue;
  return (
    <div className="flex flex-wrap gap-1.5">
      {pr && (
        <Badge
          variant="outline"
          className={
            pr.status === "merged" ? "border-chart-2/25 bg-chart-2/10 text-chart-2" : "text-muted-foreground"
          }
        >
          {pr.status === "merged" ? <GitMerge aria-hidden="true" /> : <GitPullRequest aria-hidden="true" />}#
          {pr.id} {pr.status === "merged" ? "Merged" : pr.status === "closed" ? "Closed" : "Open"}
        </Badge>
      )}
      {issue && (
        <Badge
          variant="outline"
          className={
            issue.status === "closed" ? "border-info/20 bg-info/5 text-info" : "text-muted-foreground"
          }
        >
          <CircleCheck aria-hidden="true" /> #{issue.id}{" "}
          {issue.status === "closed" ? "Resolved" : "Open issue"}
        </Badge>
      )}
      {!pr && !issue && <Badge variant="secondary">Unlinked</Badge>}
    </div>
  );
}

function SessionEvidence({ session, onOpenTrace }: { session: OutcomeSession; onOpenTrace: () => void }) {
  return (
    <>
      <SheetHeader className="border-b p-6 pr-12">
        <Badge variant="outline" className="mb-3">
          Sample evidence
        </Badge>
        <SheetTitle>Session to outcome</SheetTitle>
        <SheetDescription>{session.title}</SheetDescription>
      </SheetHeader>
      <div className="space-y-6 overflow-y-auto p-6">
        <dl className="grid grid-cols-2 gap-4 text-xs">
          <div>
            <dt className="text-muted-foreground">Session</dt>
            <dd className="mt-1 font-mono">{session.id}</dd>
          </div>
          <div>
            <dt className="text-muted-foreground">Agent spend</dt>
            <dd className="mt-1 font-mono">{money(session.costUSD)}</dd>
          </div>
          <div>
            <dt className="text-muted-foreground">Repository</dt>
            <dd className="mt-1">{session.repository}</dd>
          </div>
          <div>
            <dt className="text-muted-foreground">Agent</dt>
            <dd className="mt-1 font-mono">{session.agent}</dd>
          </div>
        </dl>
        <div className="flex flex-wrap items-center justify-between gap-3">
          <OutcomeBadges session={session} />
          <Button variant="outline" size="sm" onClick={onOpenTrace}>
            Open trace <ArrowRight className="size-3.5" />
          </Button>
        </div>
        <section aria-label="Link evidence" className="rounded-md border bg-muted/20 p-4">
          <h3 className="flex items-center gap-2 text-sm font-medium">
            <Link2 className="size-4" /> {session.evidence ? "Explicit artifact link" : "No artifact linked"}
          </h3>
          <p className="mt-2 text-xs leading-5 text-muted-foreground">
            {session.evidence?.description ??
              "This session has no recorded PR or issue identifier. Its delivery outcome is unknown."}
          </p>
          <p className="mt-2 text-xs text-muted-foreground">
            Illustrative GitHub events and trace tool results. These are sample records.
          </p>
        </section>
        <section aria-label="Outcome timeline">
          <h3 className="mb-4 text-sm font-medium">Evidence trail</h3>
          <ol className="ml-1.5 border-l pl-5">
            {session.events.map((event, index) => (
              <li key={`${event.at}-${index}`} className="relative pb-6 last:pb-0">
                <span
                  aria-hidden="true"
                  className="absolute top-1 -left-[25px] size-2 rounded-full border-2 border-background bg-chart-2"
                />
                <p className="text-sm font-medium">{event.label}</p>
                <time className="mt-1 block font-mono text-[10px] text-muted-foreground" dateTime={event.at}>
                  {new Date(event.at).toLocaleString()}
                </time>
                <p className="mt-1 text-xs leading-5 text-muted-foreground">{event.detail}</p>
              </li>
            ))}
          </ol>
        </section>
        <p className="border-t pt-4 text-xs leading-5 text-muted-foreground">
          A linked merge shows delivery associated with this session. It does not establish how much time the
          agent saved or whether the work would have shipped without it.
        </p>
      </div>
    </>
  );
}

function exportSessions(sessions: readonly OutcomeSession[]) {
  const rows = [
    [
      "Session",
      "Agent",
      "Repository",
      "Started",
      "Agent spend USD",
      "Pull request",
      "PR status",
      "Issue",
      "Issue status",
    ],
    ...sessions.map((row) => [
      row.id,
      row.agent,
      row.repository,
      row.startedAt,
      row.costUSD.toFixed(2),
      row.pullRequest?.id ?? "",
      row.pullRequest?.status ?? "",
      row.issue?.id ?? "",
      row.issue?.status ?? "",
    ]),
  ];
  const csv = rows
    .map((row) => row.map((value) => `"${String(value).replaceAll('"', '""')}"`).join(","))
    .join("\n");
  const url = URL.createObjectURL(new Blob([csv], { type: "text/csv;charset=utf-8" }));
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = "lens-outcomes-sample.csv";
  anchor.click();
  URL.revokeObjectURL(url);
}

function OutcomeRanking({
  title,
  rows,
  measure,
  onSelect,
}: {
  title: string;
  rows: readonly {
    name: string;
    value: number;
    segments: readonly { name: string; value: number; color: string }[];
  }[];
  measure: OutcomeMetric;
  onSelect?: (name: string) => void;
}) {
  const maximum = Math.max(...rows.map((row) => row.value), 1);
  return (
    <section aria-label={title} className="min-w-0 rounded-lg border bg-card p-4 sm:p-5">
      <div className="mb-5 flex items-center justify-between gap-3">
        <h3 className="text-lg font-medium">{title}</h3>
        <span className="text-xs text-muted-foreground">{OUTCOME_MEASURES[measure].label}</span>
      </div>
      <div className="space-y-5">
        {rows.length === 0 && <p className="py-4 text-sm text-muted-foreground">No matching activity</p>}
        {rows.map((row) => (
          <div key={row.name}>
            <div className="mb-2 flex items-center justify-between gap-3 text-sm">
              {onSelect ? (
                <Button
                  variant="link"
                  size="sm"
                  className="h-auto min-w-0 truncate p-0 text-left font-normal text-foreground"
                  onClick={() => onSelect(row.name)}
                >
                  {row.name}
                  <ChevronRight className="size-3" />
                </Button>
              ) : (
                <span className="truncate">{row.name}</span>
              )}
              <span className="shrink-0 tabular-nums">{outcomeValue(row.value, measure)}</span>
            </div>
            <div
              role="meter"
              aria-label={row.name}
              aria-valuenow={row.value}
              aria-valuemin={0}
              aria-valuemax={maximum}
              aria-valuetext={`${outcomeValue(row.value, measure)} ${OUTCOME_MEASURES[measure].unit}`}
              className="flex h-6 overflow-hidden rounded-sm bg-muted/40"
            >
              {row.segments.map((segment) => (
                <span
                  key={segment.name}
                  title={`${segment.name}: ${outcomeValue(segment.value, measure)}`}
                  style={{ width: `${(segment.value / maximum) * 100}%`, background: segment.color }}
                />
              ))}
            </div>
          </div>
        ))}
      </div>
    </section>
  );
}

export function OutcomesView({ agent }: { agent: string | null }) {
  const route = useLensRoute();
  const filters = useOutcomesRoute();
  const [now] = useState(Date.now);
  const [records] = useState(() => createOutcomeDemo(now));
  const [measure, setMeasure] = useState<OutcomeMetric>("mergedPRs");
  const summaryKey = measure === "costUSD" ? "totalCostUSD" : measure;
  const scopedAgent = filters.scope === "agent" ? agent : null;
  const sessions = useMemo(
    () =>
      filterOutcomeSessions(
        records,
        {
          agent: scopedAgent,
          repository: filters.repository,
          days: Number(filters.days),
          status: filters.status,
        },
        now,
      ),
    [records, scopedAgent, filters.repository, filters.days, filters.status, now],
  );
  const summary = summarizeOutcomes(sessions);
  const repos = repositoryOutcomes(sessions);
  const agents = agentOutcomes(sessions);
  const chartAgents = [...new Set(sessions.map((session) => session.agent))].sort();
  const colorFor = (name: string) =>
    OUTCOME_AGENT_COLORS[chartAgents.indexOf(name) % OUTCOME_AGENT_COLORS.length];
  const trend = dailyAgentOutcomeTrend(sessions, Number(filters.days), now, measure);
  const repositoryOptions = [
    { value: "all", label: "All repositories" },
    ...Array.from(
      new Set(
        records
          .filter((record) => !scopedAgent || record.agent === scopedAgent)
          .map((record) => record.repository),
      ),
    ).map((name) => ({ value: name, label: name })),
  ];
  const scopeOptions = [
    { value: "workspace", label: "Workspace" },
    { value: "agent", label: `Selected agent${agent ? `: ${agent}` : ""}` },
  ];
  const selected = sessions.find((session) => session.id === filters.session);
  const openTrace = (session: OutcomeSession) => filters.openTrace(outcomeTraceId(session), session.agent);

  if (!route.demo)
    return (
      <section aria-label="Agent outcomes" className="flex flex-1 flex-col">
        <LensPageHeader
          page="outcomes"
          title="Agent outcomes"
          description="Follow agent activity through to shipped work."
        />
        <div className="lens-page-body">
          <StateMessage
            role="status"
            icon={<GitMerge className="size-5" />}
            title="Explore the outcomes dashboard"
            description="Preview how sessions, merged pull requests, closed issues and agent spend fit together. Live outcome ingestion is not connected in this preview."
          />
          <Button className="mt-4" onClick={() => route.setDemo(true)}>
            Explore sample dashboard <ArrowRight className="size-4" />
          </Button>
        </div>
      </section>
    );

  return (
    <section aria-label="Agent outcomes" className="flex min-h-0 flex-1 flex-col">
      <LensPageHeader
        page="outcomes"
        title="Outcomes usage"
        description="Merged pull requests, closed issues and spend across your agents."
        actions={
          <Badge variant="outline" className="h-7 border-warning/30 bg-warning/5 text-foreground">
            Sample data
          </Badge>
        }
      />
      <div className="lens-page-body space-y-4">
        <div className="flex flex-wrap items-end justify-between gap-4">
          <div className="flex flex-wrap items-end gap-3">
            <label className="grid gap-1.5 text-xs">
              Usage view
              <Select
                items={scopeOptions}
                value={filters.scope}
                onValueChange={(value: typeof filters.scope | null) => value && filters.setScope(value)}
              >
                <SelectTrigger aria-label="Outcomes scope" className="h-9 min-w-48 bg-card text-sm">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {scopeOptions.map((item) => (
                    <SelectItem key={item.value} value={item.value}>
                      {item.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
            <label className="grid gap-1.5 text-xs">
              Repository
              <Select
                items={repositoryOptions}
                value={filters.repository}
                onValueChange={(value: string | null) => value && filters.setRepository(value)}
              >
                <SelectTrigger aria-label="Repository" className="h-9 min-w-44 bg-card text-sm">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {repositoryOptions.map((item) => (
                    <SelectItem key={item.value} value={item.value}>
                      {item.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
            <label className="grid gap-1.5 text-xs">
              Outcome
              <Select
                items={STATES}
                value={filters.status}
                onValueChange={(value: typeof filters.status | null) => value && filters.setStatus(value)}
              >
                <SelectTrigger aria-label="Outcome status" className="h-9 bg-card text-sm">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {STATES.map((item) => (
                    <SelectItem key={item.value} value={item.value}>
                      {item.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
            {(filters.repository !== "all" || filters.status !== "all") && (
              <Button variant="ghost" size="sm" onClick={filters.reset}>
                Reset filters
              </Button>
            )}
          </div>
          <label className="grid gap-1.5 text-xs">
            Select time range
            <Select
              items={PERIODS}
              value={filters.days}
              onValueChange={(value: typeof filters.days | null) => value && filters.setDays(value)}
            >
              <SelectTrigger aria-label="Outcomes period" className="h-9 min-w-44 bg-card text-sm">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {PERIODS.map((item) => (
                  <SelectItem key={item.value} value={item.value}>
                    {item.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </label>
        </div>
        <div className="flex flex-wrap items-center justify-between gap-3 border-b pb-2 pt-2">
          <Tabs value={measure} onValueChange={(value) => setMeasure(value as OutcomeMetric)}>
            <TabsList aria-label="Outcome metric" className="h-9 rounded-md bg-muted/60">
              {(Object.keys(OUTCOME_MEASURES) as OutcomeMetric[]).map((key) => (
                <TabsTrigger key={key} value={key} className="rounded-md px-4 text-sm data-active:bg-card">
                  {OUTCOME_MEASURES[key].label}
                </TabsTrigger>
              ))}
            </TabsList>
          </Tabs>
          <Button
            variant="outline"
            size="sm"
            disabled={!sessions.length}
            onClick={() => exportSessions(sessions)}
          >
            <Download className="size-3.5" /> Export CSV
          </Button>
        </div>
        <div className="flex flex-wrap gap-x-10 gap-y-4 py-3">
          {[
            {
              label: "Merged pull requests",
              value: summary.mergedPRs.toLocaleString(),
              detail: "Unique linked PRs",
            },
            {
              label: "Issues resolved",
              value: summary.closedIssues.toLocaleString(),
              detail: "Unique closed issues",
            },
            {
              label: "Agent spend",
              value: money(summary.totalCostUSD),
              detail: `${summary.sessions} sessions · USD`,
            },
            {
              label: "Spend / merged PR",
              value: summary.costPerMergedPR === null ? "—" : money(summary.costPerMergedPR),
              detail: "All selected spend ÷ unique merges",
            },
          ].map(({ label, value, detail }) => (
            <div key={label} role="group" aria-label={label} className="min-w-36">
              <p className="text-xs text-muted-foreground">{label}</p>
              <p className="mt-1 text-lg font-medium tabular-nums">{value}</p>
              <p className="mt-1 text-[11px] text-muted-foreground">{detail}</p>
            </div>
          ))}
        </div>
        <OutcomesTrend data={trend} agents={chartAgents} measure={measure} />
        <div className="grid gap-4 lg:grid-cols-2">
          <OutcomeRanking
            title="Top Agents"
            measure={measure}
            rows={[...agents]
              .sort((left, right) => right[summaryKey] - left[summaryKey])
              .map((item) => ({
                name: item.agent,
                value: item[summaryKey],
                segments: [{ name: item.agent, value: item[summaryKey], color: colorFor(item.agent) }],
              }))}
          />
          <OutcomeRanking
            title="Top Repositories"
            measure={measure}
            onSelect={filters.setRepository}
            rows={[...repos]
              .sort((left, right) => right[summaryKey] - left[summaryKey])
              .map((repo) => ({
                name: repo.repository,
                value: repo[summaryKey],
                segments: agentOutcomes(
                  sessions.filter((session) => session.repository === repo.repository),
                ).map((item) => ({ name: item.agent, value: item[summaryKey], color: colorFor(item.agent) })),
              }))}
          />
        </div>
        <section aria-label="Session outcomes" className="overflow-hidden rounded-lg border bg-card">
          <div className="flex flex-wrap items-center justify-between gap-3 border-b px-4 py-4 sm:px-5">
            <div>
              <h3 className="text-base font-medium">Session outcomes</h3>
              <p className="mt-1 text-xs text-muted-foreground">
                {summary.sessions} sessions · {percentage(summary.coverage)} linked to a PR or issue
              </p>
            </div>
            <Button
              variant="link"
              size="sm"
              className="h-auto p-0 text-xs"
              onClick={() => filters.setStatus("unlinked")}
            >
              Review unlinked sessions <ArrowRight className="size-3" />
            </Button>
          </div>
          {!sessions.length ? (
            <div className="p-6">
              <StateMessage
                role="status"
                icon={<GitMerge className="size-5" />}
                title="No sessions match these filters"
                description="Choose another repository, outcome or period."
              />
              <Button variant="outline" size="sm" className="mt-3" onClick={filters.reset}>
                Reset filters
              </Button>
            </div>
          ) : (
            <Table aria-label="Session outcomes">
              <TableHeader>
                <TableRow>
                  <TableHead className="pl-5">Task / session</TableHead>
                  <TableHead>Agent</TableHead>
                  <TableHead>Outcome</TableHead>
                  <TableHead className="text-right">Agent spend</TableHead>
                  <TableHead>Started</TableHead>
                  <TableHead className="pr-5 text-right">Trace</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {sessions.slice(0, 12).map((session) => (
                  <TableRow key={session.id}>
                    <TableCell className="max-w-72 pl-5">
                      <p className="truncate text-xs font-medium">{session.title}</p>
                      <p className="mt-1 font-mono text-[10px] text-muted-foreground">
                        {session.id} · {session.repository}
                      </p>
                    </TableCell>
                    <TableCell className="text-xs">{session.agent}</TableCell>
                    <TableCell>
                      <OutcomeBadges session={session} />
                    </TableCell>
                    <TableCell className="text-right text-xs tabular-nums">
                      {money(session.costUSD)}
                    </TableCell>
                    <TableCell className="text-xs text-muted-foreground">{date(session.startedAt)}</TableCell>
                    <TableCell className="pr-5 text-right">
                      <div className="flex items-center justify-end gap-1">
                        <Button
                          variant="outline"
                          size="sm"
                          className="h-7 text-xs"
                          aria-label={`Open trace for ${session.id}`}
                          onClick={() => openTrace(session)}
                        >
                          Open trace <ArrowRight className="size-3" />
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          className="h-7 text-xs"
                          aria-label={`View evidence for ${session.id}`}
                          onClick={() => filters.setSession(session.id)}
                        >
                          Evidence
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
          {sessions.length > 12 && (
            <p className="border-t px-5 py-3 text-xs text-muted-foreground">
              Showing 12 of {sessions.length} sessions. Export CSV includes all filtered sessions.
            </p>
          )}
        </section>
        <p className="text-xs leading-5 text-muted-foreground">
          Illustrative data for {scopedAgent || "all agents"}. Sessions are selected by start date; outcomes
          use their merge or closure date. Explicit links show association, not causal productivity lift. No
          estimated hours saved or financial ROI is included.
        </p>
      </div>
      <Sheet open={!!selected} onOpenChange={(open) => !open && filters.setSession(null)}>
        <SheetContent className="w-full! max-w-lg! gap-0 p-0">
          {selected && <SessionEvidence session={selected} onOpenTrace={() => openTrace(selected)} />}
        </SheetContent>
      </Sheet>
    </section>
  );
}
