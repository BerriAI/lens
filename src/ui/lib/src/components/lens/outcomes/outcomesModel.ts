export type OutcomeStatus = "all" | "shipped" | "in_progress" | "unlinked";
export type OutcomeMetric = "mergedPRs" | "closedIssues" | "costUSD";
export type OutcomeRowStatus =
  | Exclude<OutcomeStatus, "all">
  | "closed_without_merge";

export interface OutcomePullRequest {
  id: number;
  title: string;
  status: "merged" | "open" | "closed";
  mergedAt?: string;
}

export interface OutcomeIssue {
  id: number;
  title: string;
  status: "closed" | "open";
  closedAt?: string;
}

export interface OutcomeSession {
  id: string;
  agent: string;
  repository: string;
  title: string;
  startedAt: string;
  finishedAt: string;
  costUSD: number;
  pullRequest: OutcomePullRequest | null;
  issue: OutcomeIssue | null;
  evidence: {
    method: "explicit";
    source: "GitHub webhook";
    description: string;
  } | null;
  events: { at: string; label: string; detail: string }[];
}

export interface OutcomeFilters {
  agent?: string | null;
  repository?: string | null;
  days: number;
  status: OutcomeStatus;
}

export interface OutcomeSummary {
  sessions: number;
  totalCostUSD: number;
  mergedPRs: number;
  closedIssues: number;
  linkedSessions: number;
  coverage: number;
  costPerMergedPR: number | null;
}

export interface RepositoryOutcomes extends OutcomeSummary {
  repository: string;
  spendShare: number;
}

export interface OutcomeTrendPoint {
  date: string;
  sessions: number;
  mergedPRs: number;
  closedIssues: number;
  costUSD: number;
}

export interface OutcomeAgentTrendPoint {
  date: string;
  values: Record<string, number>;
}

export interface AgentOutcomes extends OutcomeSummary {
  agent: string;
  spendShare: number;
}

const DAY_MS = 86_400_000;
const money = (value: number) => Math.round(value * 100) / 100;
const linked = (session: OutcomeSession) =>
  session.pullRequest !== null || session.issue !== null;
const eventKey = (session: OutcomeSession, id: number) =>
  `${session.repository}#${id}`;

function creditedEvents(
  sessions: readonly OutcomeSession[],
  metric: Exclude<OutcomeMetric, "costUSD">,
) {
  const events = new Map<string, { agent: string; at: string }>();
  for (const session of sessions.toSorted(
    (left, right) =>
      left.startedAt.localeCompare(right.startedAt) ||
      left.id.localeCompare(right.id),
  )) {
    const artifact =
      metric === "mergedPRs" ? session.pullRequest : session.issue;
    const occurred =
      metric === "mergedPRs"
        ? session.pullRequest?.mergedAt
        : session.issue?.closedAt;
    const completed =
      metric === "mergedPRs"
        ? artifact?.status === "merged"
        : artifact?.status === "closed";
    if (!artifact || !occurred || !completed) continue;
    const key = eventKey(session, artifact.id);
    if (!events.has(key))
      events.set(key, { agent: session.agent, at: occurred });
  }
  return events;
}

export function agentOutcomes(
  sessions: readonly OutcomeSession[],
): AgentOutcomes[] {
  const total = summarizeOutcomes(sessions).totalCostUSD;
  const merged = [...creditedEvents(sessions, "mergedPRs").values()];
  const closed = [...creditedEvents(sessions, "closedIssues").values()];
  return [...new Set(sessions.map((session) => session.agent))]
    .map((agent) => {
      const summary = summarizeOutcomes(
        sessions.filter((session) => session.agent === agent),
      );
      const mergedPRs = merged.filter((event) => event.agent === agent).length;
      return {
        ...summary,
        agent,
        mergedPRs,
        closedIssues: closed.filter((event) => event.agent === agent).length,
        costPerMergedPR: mergedPRs
          ? money(summary.totalCostUSD / mergedPRs)
          : null,
        spendShare: total ? summary.totalCostUSD / total : 0,
      };
    })
    .sort(
      (left, right) =>
        right.totalCostUSD - left.totalCostUSD ||
        left.agent.localeCompare(right.agent),
    );
}

export function dailyAgentOutcomeTrend(
  sessions: readonly OutcomeSession[],
  days: number,
  now: number,
  metric: OutcomeMetric,
): OutcomeAgentTrendPoint[] {
  const agents = [...new Set(sessions.map((session) => session.agent))].sort();
  const events =
    metric === "costUSD" ? [] : [...creditedEvents(sessions, metric).values()];
  return dailyOutcomeTrend(sessions, days, now).map(({ date }) => ({
    date,
    values: Object.fromEntries(
      agents.map((agent) => [
        agent,
        metric === "costUSD"
          ? money(
              sessions
                .filter(
                  (session) =>
                    session.agent === agent &&
                    session.startedAt.slice(0, 10) === date,
                )
                .reduce((sum, session) => sum + session.costUSD, 0),
            )
          : events.filter(
              (event) =>
                event.agent === agent && event.at.slice(0, 10) === date,
            ).length,
      ]),
    ),
  }));
}

export function outcomeStatus(session: OutcomeSession): OutcomeRowStatus {
  if (
    session.pullRequest?.status === "merged" ||
    session.issue?.status === "closed"
  )
    return "shipped";
  if (
    session.pullRequest?.status === "open" ||
    session.issue?.status === "open"
  )
    return "in_progress";
  return linked(session) ? "closed_without_merge" : "unlinked";
}

export function filterOutcomeSessions(
  sessions: readonly OutcomeSession[],
  filters: OutcomeFilters,
  now: number,
): OutcomeSession[] {
  const start = now - filters.days * DAY_MS;
  return sessions.filter((session) => {
    const started = Date.parse(session.startedAt);
    return (
      started >= start &&
      started < now &&
      (!filters.agent ||
        filters.agent === "all" ||
        session.agent === filters.agent) &&
      (!filters.repository ||
        filters.repository === "all" ||
        session.repository === filters.repository) &&
      (filters.status === "all" || outcomeStatus(session) === filters.status)
    );
  });
}

export function summarizeOutcomes(
  sessions: readonly OutcomeSession[],
): OutcomeSummary {
  const merged = new Set(
    sessions.flatMap((session) =>
      session.pullRequest?.status === "merged"
        ? [eventKey(session, session.pullRequest.id)]
        : [],
    ),
  );
  const closed = new Set(
    sessions.flatMap((session) =>
      session.issue?.status === "closed"
        ? [eventKey(session, session.issue.id)]
        : [],
    ),
  );
  const totalCostUSD = money(
    sessions.reduce((sum, session) => sum + session.costUSD, 0),
  );
  const linkedSessions = sessions.filter(linked).length;
  return {
    sessions: sessions.length,
    totalCostUSD,
    mergedPRs: merged.size,
    closedIssues: closed.size,
    linkedSessions,
    coverage: sessions.length ? linkedSessions / sessions.length : 0,
    costPerMergedPR: merged.size ? money(totalCostUSD / merged.size) : null,
  };
}

export function repositoryOutcomes(
  sessions: readonly OutcomeSession[],
): RepositoryOutcomes[] {
  const total = summarizeOutcomes(sessions).totalCostUSD;
  return [...new Set(sessions.map((session) => session.repository))]
    .map((repository) => {
      const summary = summarizeOutcomes(
        sessions.filter((session) => session.repository === repository),
      );
      return {
        repository,
        ...summary,
        spendShare: total ? summary.totalCostUSD / total : 0,
      };
    })
    .sort(
      (left, right) =>
        right.totalCostUSD - left.totalCostUSD ||
        left.repository.localeCompare(right.repository),
    );
}

export function dailyOutcomeTrend(
  sessions: readonly OutcomeSession[],
  days: number,
  now: number,
): OutcomeTrendPoint[] {
  const lastDay = Math.floor(now / DAY_MS) * DAY_MS;
  return Array.from({ length: days }, (_, index) => {
    const date = new Date(lastDay - (days - index - 1) * DAY_MS)
      .toISOString()
      .slice(0, 10);
    const started = sessions.filter(
      (session) => session.startedAt.slice(0, 10) === date,
    );
    const merged = new Set(
      sessions.flatMap((session) =>
        session.pullRequest?.status === "merged" &&
        session.pullRequest.mergedAt?.slice(0, 10) === date
          ? [eventKey(session, session.pullRequest.id)]
          : [],
      ),
    );
    const closed = new Set(
      sessions.flatMap((session) =>
        session.issue?.status === "closed" &&
        session.issue.closedAt?.slice(0, 10) === date
          ? [eventKey(session, session.issue.id)]
          : [],
      ),
    );
    return {
      date,
      sessions: started.length,
      mergedPRs: merged.size,
      closedIssues: closed.size,
      costUSD: money(
        started.reduce((sum, session) => sum + session.costUSD, 0),
      ),
    };
  });
}

const DEMO_AGENTS = [
  "release_agent",
  "research_agent",
  "support_agent",
] as const;
const DEMO_REPOSITORIES = [
  "acme/platform",
  "acme/console",
  "acme/services",
] as const;
const DEMO_TASKS = [
  "Add webhook retry backoff",
  "Fix workspace permissions",
  "Improve model routing fallback",
  "Add trace search pagination",
  "Resolve duplicate notification delivery",
  "Refresh API client types",
  "Improve cache key isolation",
  "Fix task cancellation handling",
  "Add request budget validation",
] as const;

export function createOutcomeDemo(now: number): OutcomeSession[] {
  return Array.from({ length: 144 }, (_, index) => {
    const group = Math.floor(index / 2);
    const attempt = index % 2;
    const started = now - (3 + group * 12 - attempt * 0.35) * 3_600_000;
    const finished = started + (12 + (group % 19)) * 60_000;
    const occurred = now - (1 + group * 12) * 3_600_000;
    const title = DEMO_TASKS[group % DEMO_TASKS.length];
    const repository =
      DEMO_REPOSITORIES[Math.floor(group / 3) % DEMO_REPOSITORIES.length];
    const isLinked = group % 10 !== 9;
    const merged = group % 10 < ([8, 5, 3] as const)[group % DEMO_AGENTS.length];
    const pullRequest: OutcomePullRequest | null = isLinked
      ? {
          id: 480 + group,
          title,
          status: merged ? "merged" : group % 10 === 8 ? "closed" : "open",
          ...(merged ? { mergedAt: new Date(occurred).toISOString() } : {}),
        }
      : null;
    const issue: OutcomeIssue | null =
      isLinked && group % 4 === 0
        ? {
            id: 210 + group,
            title: `Track: ${title.toLowerCase()}`,
            status: merged ? "closed" : "open",
            ...(merged
              ? { closedAt: new Date(occurred + 60_000).toISOString() }
              : {}),
          }
        : null;
    const events: OutcomeSession["events"] = [
      {
        at: new Date(started).toISOString(),
        label: "Session started",
        detail: `Task: ${title}`,
      },
      {
        at: new Date(started + 4 * 60_000).toISOString(),
        label: "Changes prepared",
        detail: "Edited implementation and ran focused checks",
      },
      {
        at: new Date(finished).toISOString(),
        label: "Session completed",
        detail: "Trace recorded with model usage and tool calls",
      },
    ];
    if (pullRequest)
      events.push({
        at: new Date(finished + 60_000).toISOString(),
        label: `PR #${pullRequest.id} linked`,
        detail:
          "Explicit repository and PR identifiers recorded on the session",
      });
    if (merged)
      events.push({
        at: new Date(occurred).toISOString(),
        label: "Pull request merged",
        detail: "GitHub merge event matched the linked PR identifier",
      });
    if (issue?.closedAt)
      events.push({
        at: issue.closedAt,
        label: `Issue #${issue.id} closed`,
        detail: "GitHub close event matched the linked issue identifier",
      });
    return {
      id: `demo-session-${String(index + 1).padStart(3, "0")}`,
      agent: DEMO_AGENTS[group % DEMO_AGENTS.length],
      repository,
      title: `${title}${attempt ? " · follow-up" : ""}`,
      startedAt: new Date(started).toISOString(),
      finishedAt: new Date(finished).toISOString(),
      costUSD: money(3.2 + (group % 13) * 1.84 + attempt * 0.73),
      pullRequest,
      issue,
      evidence: isLinked
        ? {
            method: "explicit",
            source: "GitHub webhook",
            description:
              "Session metadata links this repository and artifact ID; a matching GitHub event confirms its state. This shows association, not causal productivity lift.",
          }
        : null,
      events,
    };
  });
}
