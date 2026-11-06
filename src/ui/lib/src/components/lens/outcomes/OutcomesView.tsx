"use client";

import { useMemo, useState } from "react";
import {
  ArrowRight,
  CircleCheck,
  Download,
  GitMerge,
  GitPullRequest,
  Link2,
} from "lucide-react";
import { Button } from "../../ui/button";
import { Badge } from "../../ui/badge";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  TableFooter,
} from "../../ui/table";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "../../ui/sheet";
import { StateMessage } from "../../shared/StateMessage";
import { LensPageHeader } from "../ui/LensPageHeader";
import { useLensRoute, useOutcomesRoute } from "../route";
import {
  CODING_AGENTS,
  createOutcomeDemo,
  filterOutcomeSessions,
  summarizeOutcomes,
  userOutcomes,
  taskActivity,
  taskBreakdown,
  type OutcomeSession,
} from "./outcomesModel";
import { outcomeTraceId } from "./outcomeTrace";
import { CodingOutcomeFilters } from "./CodingOutcomeFilters";
import { TaskActivity, taskColor } from "./TaskActivity";

const money = (value: number) =>
  value.toLocaleString("en-US", { style: "currency", currency: "USD" });
const percentage = (value: number) => `${Math.round(value * 100)}%`;
const date = (value: string) =>
  new Date(value).toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
  });
function OutcomeBadges({ session }: { session: OutcomeSession }) {
  const pr = session.pullRequest;
  const issue = session.issue;
  return (
    <div className="flex flex-wrap gap-1.5">
      {pr && (
        <Badge
          variant="outline"
          className={
            pr.status === "merged"
              ? "border-chart-2/25 bg-chart-2/10 text-chart-2"
              : "text-muted-foreground"
          }
        >
          {pr.status === "merged" ? (
            <GitMerge aria-hidden="true" />
          ) : (
            <GitPullRequest aria-hidden="true" />
          )}
          #{pr.id}{" "}
          {pr.status === "merged"
            ? "Merged"
            : pr.status === "closed"
              ? "Closed"
              : "Open"}
        </Badge>
      )}
      {issue && (
        <Badge
          variant="outline"
          className={
            issue.status === "closed"
              ? "border-info/20 bg-info/5 text-info"
              : "text-muted-foreground"
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

function SessionEvidence({
  session,
  onOpenTrace,
}: {
  session: OutcomeSession;
  onOpenTrace: () => void;
}) {
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
        <section
          aria-label="Link evidence"
          className="rounded-md border bg-muted/20 p-4"
        >
          <h3 className="flex items-center gap-2 text-sm font-medium">
            <Link2 className="size-4" />{" "}
            {session.evidence ? "Explicit artifact link" : "No artifact linked"}
          </h3>
          <p className="mt-2 text-xs leading-5 text-muted-foreground">
            {session.evidence?.description ??
              "This session has no recorded PR or issue identifier. Its delivery outcome is unknown."}
          </p>
          <p className="mt-2 text-xs text-muted-foreground">
            Illustrative GitHub events and trace tool results. These are sample
            records.
          </p>
        </section>
        <section aria-label="Outcome timeline">
          <h3 className="mb-4 text-sm font-medium">Evidence trail</h3>
          <ol className="ml-1.5 border-l pl-5">
            {session.events.map((event, index) => (
              <li
                key={`${event.at}-${index}`}
                className="relative pb-6 last:pb-0"
              >
                <span
                  aria-hidden="true"
                  className="absolute top-1 -left-[25px] size-2 rounded-full border-2 border-background bg-chart-2"
                />
                <p className="text-sm font-medium">{event.label}</p>
                <time
                  className="mt-1 block font-mono text-[10px] text-muted-foreground"
                  dateTime={event.at}
                >
                  {new Date(event.at).toLocaleString()}
                </time>
                <p className="mt-1 text-xs leading-5 text-muted-foreground">
                  {event.detail}
                </p>
              </li>
            ))}
          </ol>
        </section>
        <p className="border-t pt-4 text-xs leading-5 text-muted-foreground">
          A linked merge shows delivery associated with this session. It does
          not establish how much time the agent saved or whether the work would
          have shipped without it.
        </p>
      </div>
    </>
  );
}

function exportSessions(sessions: readonly OutcomeSession[]) {
  const rows = [
    [
      "Session",
      "User",
      "Agent",
      "Coding agent",
      "Team",
      "Task activity",
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
      row.user,
      row.agent,
      CODING_AGENTS.find((agent) => agent.value === row.codingAgent)?.label ??
        row.codingAgent,
      row.team,
      row.taskCategory,
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
    .map((row) =>
      row.map((value) => `"${String(value).replaceAll('"', '""')}"`).join(","),
    )
    .join("\n");
  const url = URL.createObjectURL(
    new Blob([csv], { type: "text/csv;charset=utf-8" }),
  );
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = "lens-coding-outcomes-sample.csv";
  anchor.click();
  URL.revokeObjectURL(url);
}

function PersonSessions({
  user,
  sessions,
  onOpenTrace,
  onEvidence,
}: {
  user: string;
  sessions: readonly OutcomeSession[];
  onOpenTrace: (session: OutcomeSession) => void;
  onEvidence: (id: string) => void;
}) {
  return (
    <>
      <SheetHeader className="border-b p-5 pr-12">
        <SheetTitle>Sessions for {user}</SheetTitle>
        <SheetDescription>
          {sessions.length} sessions in the selected period
        </SheetDescription>
      </SheetHeader>
      <div className="min-h-0 overflow-auto px-5">
        <Table aria-label="Session outcomes">
          <TableHeader>
            <TableRow>
              <TableHead>Task</TableHead>
              <TableHead>Outcome</TableHead>
              <TableHead className="text-right">Spend</TableHead>
              <TableHead>Started</TableHead>
              <TableHead className="text-right">Trace</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {sessions.map((session) => (
              <TableRow key={session.id}>
                <TableCell className="max-w-64">
                  <p className="truncate text-xs font-medium">
                    {session.title}
                  </p>
                  <p className="mt-1 text-[11px] text-muted-foreground">
                    {
                      CODING_AGENTS.find(
                        (agent) => agent.value === session.codingAgent,
                      )?.label
                    }{" "}
                    · {session.team} · {session.id}
                  </p>
                </TableCell>
                <TableCell>
                  <OutcomeBadges session={session} />
                </TableCell>
                <TableCell className="text-right text-xs tabular-nums">
                  {money(session.costUSD)}
                </TableCell>
                <TableCell className="text-xs text-muted-foreground">
                  {date(session.startedAt)}
                </TableCell>
                <TableCell>
                  <div className="flex items-center justify-end gap-1">
                    <Button
                      variant="outline"
                      size="sm"
                      className="h-7 text-xs"
                      aria-label={`Open trace for ${session.id}`}
                      onClick={() => onOpenTrace(session)}
                    >
                      Open trace <ArrowRight className="size-3" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-7 text-xs"
                      aria-label={`View evidence for ${session.id}`}
                      onClick={() => onEvidence(session.id)}
                    >
                      Evidence
                    </Button>
                  </div>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
      <div className="mt-auto border-t p-5">
        <Button
          variant="outline"
          size="sm"
          onClick={() => exportSessions(sessions)}
        >
          <Download className="size-3.5" /> Export sessions
        </Button>
      </div>
    </>
  );
}

export function OutcomesView({ agent }: { agent: string | null }) {
  const route = useLensRoute();
  const filters = useOutcomesRoute();
  const [now] = useState(Date.now);
  const [records] = useState(() => createOutcomeDemo(now));
  const [selectedUser, setSelectedUser] = useState<string | null>(null);
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
          team: filters.team,
          codingAgent: filters.codingAgent,
          from: filters.from,
          to: filters.to,
        },
        now,
      ),
    [
      records,
      scopedAgent,
      filters.repository,
      filters.days,
      filters.status,
      filters.team,
      filters.codingAgent,
      filters.from,
      filters.to,
      now,
    ],
  );
  const summary = summarizeOutcomes(sessions);
  const people = userOutcomes(sessions);
  const categories = taskBreakdown(sessions);
  const activity = taskActivity(
    sessions,
    Number(filters.days),
    now,
    filters.from,
    filters.to,
  );
  const maximumSpend = Math.max(
    ...people.map((person) => person.totalCostUSD),
    1,
  );
  const prSummary = summarizeOutcomes(
    sessions.filter((session) => session.pullRequest !== null),
  );
  const selected = sessions.find((session) => session.id === filters.session);
  const teams = [...new Set(records.map((session) => session.team))].sort();
  const filtered =
    filters.repository !== "all" ||
    filters.status !== "all" ||
    filters.team !== "all" ||
    filters.codingAgent !== "all" ||
    filters.from !== null ||
    filters.to !== null ||
    filters.scope !== "workspace";
  const openTrace = (session: OutcomeSession) =>
    filters.openTrace(outcomeTraceId(session), session.agent);

  if (!route.demo)
    return (
      <section
        aria-label="Coding agent outcomes"
        className="flex flex-1 flex-col"
      >
        <LensPageHeader
          page="outcomes"
          title="Coding agent outcomes"
          description="Task activity, merged PRs and closed issues."
        />
        <div className="lens-page-body">
          <StateMessage
            role="status"
            icon={<GitMerge className="size-5" />}
            title="Explore coding agent outcomes"
            description="Preview coding-agent activity and delivery. Live outcome ingestion is not connected in this preview."
          />
          <Button className="mt-4" onClick={() => route.setDemo(true)}>
            Explore sample dashboard <ArrowRight className="size-4" />
          </Button>
        </div>
      </section>
    );

  return (
    <section
      aria-label="Coding agent outcomes"
      className="flex min-h-0 flex-1 flex-col"
    >
      <div className="lens-page-body space-y-5">
        <header className="flex flex-wrap items-center justify-between gap-4">
          <div className="flex items-center gap-3">
            <h2 className="text-xl font-semibold tracking-tight">
              Coding agent outcomes
            </h2>
            <span className="text-[11px] text-muted-foreground">
              Sample data
            </span>
          </div>
          <CodingOutcomeFilters teams={teams} now={now} />
        </header>
        {filtered && (
          <div className="flex flex-wrap items-center gap-3 text-xs text-muted-foreground">
            {(filters.repository !== "all" ||
              filters.status !== "all" ||
              filters.scope === "agent") && (
              <span>
                {[
                  filters.repository !== "all" && filters.repository,
                  filters.status !== "all" &&
                    filters.status.replaceAll("_", " "),
                  filters.scope === "agent" && agent,
                ]
                  .filter(Boolean)
                  .join(" · ")}
              </span>
            )}
            <Button
              variant="link"
              size="sm"
              className="h-auto p-0 text-xs"
              onClick={filters.reset}
            >
              Reset filters
            </Button>
          </div>
        )}
        <dl className="grid grid-cols-2 gap-y-4 border-y py-4 sm:grid-cols-4">
          {[
            {
              label: "Sessions",
              display: "Sessions",
              value: summary.sessions.toLocaleString(),
            },
            {
              label: "Merged pull requests",
              display: "PRs merged",
              value: summary.mergedPRs.toLocaleString(),
            },
            {
              label: "Closed issues",
              display: "Issues closed",
              value: summary.closedIssues.toLocaleString(),
            },
            {
              label: "Agent spend",
              display: "Agent spend",
              value: money(summary.totalCostUSD),
            },
          ].map((metric) => (
            <div key={metric.label} role="group" aria-label={metric.label}>
              <dt className="text-xs text-muted-foreground">
                {metric.display}
              </dt>
              <dd className="mt-1 text-xl font-medium tabular-nums">
                {metric.value}
              </dd>
            </div>
          ))}
        </dl>
        <TaskActivity data={activity} categories={categories} />
        <div className="flex items-center justify-between border-b pb-2">
          <h3 className="text-sm font-medium">People</h3>
          <Button
            variant="ghost"
            size="sm"
            className="h-7 text-xs"
            disabled={!sessions.length}
            onClick={() => exportSessions(sessions)}
          >
            <Download className="size-3" /> Export CSV
          </Button>
        </div>
        {people.length === 0 ? (
          <StateMessage
            role="status"
            icon={<GitMerge className="size-5" />}
            title="No sessions match these filters"
            description="Change the team, coding agent or time range."
          />
        ) : (
          <Table aria-label="Usage by person" className="min-w-[890px] text-xs">
            <TableHeader>
              <TableRow className="text-[11px] hover:bg-transparent [&_th]:h-8 [&_th]:font-normal [&_th]:text-muted-foreground">
                <TableHead scope="col" className="w-32 pl-0">
                  Person
                </TableHead>
                <TableHead scope="col" className="w-44">
                  Agent spend
                </TableHead>
                <TableHead scope="col" className="text-right">
                  Sessions
                </TableHead>
                <TableHead scope="col" className="text-right">
                  PRs merged
                </TableHead>
                <TableHead scope="col" className="text-right">
                  Issues closed
                </TableHead>
                <TableHead scope="col" className="text-right">
                  $ / merge
                </TableHead>
                <TableHead scope="col" className="pl-5">
                  Task activity
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {people.map((person, index) => {
                const teamNames = [
                  ...new Set(
                    sessions
                      .filter((session) => session.user === person.user)
                      .map((session) => session.team),
                  ),
                ].join(", ");
                const mix = person.taskKinds
                  .map(
                    (task) => `${task.kind} ${percentage(task.sessionShare)}`,
                  )
                  .join(", ");
                return (
                  <TableRow key={person.user} aria-label={person.user}>
                    <TableCell className="py-3 pl-0">
                      <div className="flex items-start gap-2">
                        <span className="w-3 pt-0.5 text-[10px] tabular-nums text-muted-foreground">
                          {index + 1}
                        </span>
                        <div>
                          <Button
                            variant="link"
                            title={person.user}
                            className="h-auto p-0 text-xs font-normal text-foreground"
                            aria-label={`View sessions for ${person.user}`}
                            onClick={() => setSelectedUser(person.user)}
                          >
                            {person.user.split("@")[0]}
                          </Button>
                          <p className="mt-1 text-[10px] text-muted-foreground">
                            {teamNames}
                          </p>
                        </div>
                      </div>
                    </TableCell>
                    <TableCell>
                      <div className="flex items-center gap-2 tabular-nums">
                        <div
                          role="img"
                          aria-label={`${person.user}: ${money(person.prSpendUSD)} PR work, ${money(person.nonPRSpendUSD)} other work`}
                          className="flex h-3 w-16 shrink-0"
                        >
                          <span
                            className="bg-primary"
                            style={{
                              width: `${(person.prSpendUSD / maximumSpend) * 100}%`,
                            }}
                          />
                          <span
                            className="bg-muted-foreground/40"
                            style={{
                              width: `${(person.nonPRSpendUSD / maximumSpend) * 100}%`,
                            }}
                          />
                        </div>
                        {money(person.totalCostUSD)}
                      </div>
                    </TableCell>
                    <TableCell className="text-right tabular-nums">
                      {person.sessions}
                    </TableCell>
                    <TableCell className="text-right tabular-nums">
                      {person.mergedPRs}
                    </TableCell>
                    <TableCell className="text-right tabular-nums">
                      {person.closedIssues}
                    </TableCell>
                    <TableCell className="text-right tabular-nums">
                      {person.prCostPerMerge === null
                        ? "–"
                        : money(person.prCostPerMerge)}
                    </TableCell>
                    <TableCell className="max-w-64 pl-5" title={mix}>
                      <div
                        role="img"
                        aria-label={`${person.user} task activity: ${mix}`}
                        className="flex h-2 w-full max-w-48 overflow-hidden rounded-xs"
                      >
                        {person.taskKinds.map((task) => (
                          <span
                            key={task.kind}
                            style={{
                              width: `${task.sessionShare * 100}%`,
                              backgroundColor: taskColor(task.kind),
                            }}
                          />
                        ))}
                      </div>
                      <p className="mt-1.5 truncate text-[10px] text-muted-foreground">
                        {person.taskKinds
                          .slice(0, 2)
                          .map(
                            (task) =>
                              `${task.kind} ${percentage(task.sessionShare)}`,
                          )
                          .join(", ")}
                      </p>
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
            <TableFooter className="bg-transparent text-xs">
              <TableRow className="hover:bg-transparent">
                <TableCell className="py-3 pl-0">Total</TableCell>
                <TableCell className="tabular-nums">
                  {money(summary.totalCostUSD)}
                </TableCell>
                <TableCell className="text-right tabular-nums">
                  {summary.sessions}
                </TableCell>
                <TableCell className="text-right tabular-nums">
                  {summary.mergedPRs}
                </TableCell>
                <TableCell className="text-right tabular-nums">
                  {summary.closedIssues}
                </TableCell>
                <TableCell className="text-right tabular-nums">
                  {prSummary.costPerMergedPR === null
                    ? "–"
                    : money(prSummary.costPerMergedPR)}
                </TableCell>
                <TableCell />
              </TableRow>
            </TableFooter>
          </Table>
        )}
        <p className="text-[11px] leading-5 text-muted-foreground">
          Dates filter session starts. PRs and issues count once per person;
          totals count unique artifacts. Cost per merge uses PR-linked spend.
          Task activity includes all sessions.
        </p>
      </div>
      <Sheet
        open={!!selected || selectedUser !== null}
        onOpenChange={(open) => {
          if (!open) {
            setSelectedUser(null);
            filters.setSession(null);
          }
        }}
      >
        <SheetContent className="w-full! max-w-4xl! gap-0 p-0">
          {selected ? (
            <SessionEvidence
              session={selected}
              onOpenTrace={() => openTrace(selected)}
            />
          ) : (
            selectedUser && (
              <PersonSessions
                user={selectedUser}
                sessions={sessions.filter(
                  (session) => session.user === selectedUser,
                )}
                onOpenTrace={openTrace}
                onEvidence={filters.setSession}
              />
            )
          )}
        </SheetContent>
      </Sheet>
    </section>
  );
}
