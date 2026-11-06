export const CODING_AGENTS = [
  { value: "codex", label: "Codex" },
  { value: "claude_code", label: "Claude Code" },
] as const;
export type CodingAgent = (typeof CODING_AGENTS)[number]["value"];
export const OUTCOME_TEAMS = [
  "Platform",
  "Product",
  "Developer experience",
] as const;

export type OutcomeStatus = "all" | "shipped" | "in_progress" | "unlinked";
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
  user: string;
  team: string;
  codingAgent: CodingAgent;
  taskCategory: string;
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
  team?: string | null;
  codingAgent?: CodingAgent | "all" | null;
  from?: string | null;
  to?: string | null;
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

export interface TaskKindSummary {
  kind: string;
  sessions: number;
  sessionShare: number;
}

export interface TaskActivityPoint {
  date: string;
  endDate: string;
  counts: Record<string, number>;
  sessions: number;
}

export interface UserOutcomes {
  user: string;
  sessions: number;
  totalCostUSD: number;
  spendShare: number;
  prSessions: number;
  prSpendUSD: number;
  mergedPRs: number;
  prCostPerMerge: number | null;
  nonPRSessions: number;
  nonPRSpendUSD: number;
  nonPRShare: number;
  nonPRTaskKinds: TaskKindSummary[];
  taskKinds: TaskKindSummary[];
  closedIssues: number;
}

export function userOutcomes(
  sessions: readonly OutcomeSession[],
): UserOutcomes[] {
  const totalSpend = summarizeOutcomes(sessions).totalCostUSD;
  return [...new Set(sessions.map((session) => session.user))]
    .map((user) => {
      const owned = sessions.filter((session) => session.user === user);
      const prSessions = owned.filter(
        (session) => session.pullRequest !== null,
      );
      const nonPRSessions = owned.filter(
        (session) => session.pullRequest === null,
      );
      const summary = summarizeOutcomes(owned);
      const totalCostUSD = summary.totalCostUSD;
      const pr = summarizeOutcomes(prSessions);
      const nonPRSpendUSD = summarizeOutcomes(nonPRSessions).totalCostUSD;
      const nonPRTaskKinds = taskBreakdown(nonPRSessions);
      return {
        user,
        sessions: owned.length,
        totalCostUSD,
        spendShare: totalSpend ? totalCostUSD / totalSpend : 0,
        prSessions: prSessions.length,
        prSpendUSD: pr.totalCostUSD,
        mergedPRs: pr.mergedPRs,
        prCostPerMerge: pr.costPerMergedPR,
        nonPRSessions: nonPRSessions.length,
        nonPRSpendUSD,
        nonPRShare: totalCostUSD ? nonPRSpendUSD / totalCostUSD : 0,
        nonPRTaskKinds,
        taskKinds: taskBreakdown(owned),
        closedIssues: summary.closedIssues,
      };
    })
    .sort(
      (left, right) =>
        right.totalCostUSD - left.totalCostUSD ||
        left.user.localeCompare(right.user),
    );
}

export function taskBreakdown(
  sessions: readonly OutcomeSession[],
): TaskKindSummary[] {
  return [...new Set(sessions.map((session) => session.taskCategory))]
    .map((kind) => {
      const count = sessions.filter(
        (session) => session.taskCategory === kind,
      ).length;
      return { kind, sessions: count, sessionShare: count / sessions.length };
    })
    .sort(
      (left, right) =>
        right.sessions - left.sessions || left.kind.localeCompare(right.kind),
    );
}

const DAY_MS = 86_400_000;
const money = (value: number) => Math.round(value * 100) / 100;
const linked = (session: OutcomeSession) =>
  session.pullRequest !== null || session.issue !== null;
const eventKey = (session: OutcomeSession, id: number) =>
  `${session.repository}#${id}`;

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

export function outcomeDateStart(value: string): number | null {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) return null;
  const parsed = Date.parse(`${value}T00:00:00Z`);
  if (
    !Number.isFinite(parsed) ||
    new Date(parsed).toISOString().slice(0, 10) !== value
  )
    return null;
  return parsed;
}

function selectedWindow(
  days: number,
  now: number,
  from?: string | null,
  to?: string | null,
) {
  const toDay = to ? outcomeDateStart(to) : null;
  const fromDay = from ? outcomeDateStart(from) : null;
  if ((to && toDay === null) || (from && fromDay === null)) return null;
  const endMs = toDay === null ? now : toDay + DAY_MS;
  const startMs = fromDay === null ? endMs - days * DAY_MS : fromDay;
  if (!Number.isFinite(startMs) || !Number.isFinite(endMs) || startMs >= endMs)
    return null;
  return { startMs, endMs };
}

export function filterOutcomeSessions(
  sessions: readonly OutcomeSession[],
  filters: OutcomeFilters,
  now: number,
): OutcomeSession[] {
  const window = selectedWindow(filters.days, now, filters.from, filters.to);
  if (!window) return [];
  return sessions.filter((session) => {
    const started = Date.parse(session.startedAt);
    return (
      started >= window.startMs &&
      started < window.endMs &&
      (!filters.agent ||
        filters.agent === "all" ||
        session.agent === filters.agent) &&
      (!filters.repository ||
        filters.repository === "all" ||
        session.repository === filters.repository) &&
      (!filters.team ||
        filters.team === "all" ||
        session.team === filters.team) &&
      (!filters.codingAgent ||
        filters.codingAgent === "all" ||
        session.codingAgent === filters.codingAgent) &&
      (filters.status === "all" || outcomeStatus(session) === filters.status)
    );
  });
}

export function taskActivity(
  sessions: readonly OutcomeSession[],
  days: number,
  now: number,
  from?: string | null,
  to?: string | null,
): TaskActivityPoint[] {
  const window = selectedWindow(days, now, from, to);
  if (!window) return [];
  const rows = sessions.filter((session) => {
    const at = Date.parse(session.startedAt);
    return at >= window.startMs && at < window.endMs;
  });
  const firstDay = Math.floor(window.startMs / DAY_MS) * DAY_MS;
  const lastDay = Math.floor((window.endMs - 1) / DAY_MS) * DAY_MS;
  const calendarDays = (lastDay - firstDay) / DAY_MS + 1;
  const bucketDays = Math.ceil(calendarDays / 366);
  const kinds = taskBreakdown(rows).map((task) => task.kind);
  const buckets = Array.from(
    { length: Math.ceil(calendarDays / bucketDays) },
    (_, index) => ({
      date: new Date(firstDay + index * bucketDays * DAY_MS)
        .toISOString()
        .slice(0, 10),
      endDate: new Date(
        Math.min(lastDay, firstDay + ((index + 1) * bucketDays - 1) * DAY_MS),
      )
        .toISOString()
        .slice(0, 10),
      counts: Object.fromEntries(kinds.map((kind) => [kind, 0])),
      sessions: 0,
    }),
  );
  for (const row of rows) {
    const index = Math.floor(
      (Date.parse(row.startedAt) - firstDay) / (bucketDays * DAY_MS),
    );
    buckets[index].counts[row.taskCategory] += 1;
    buckets[index].sessions += 1;
  }
  return buckets;
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
const DEMO_USERS = [
  "alex@example.test",
  "jordan@example.test",
  "sam@example.test",
  "taylor@example.test",
  "casey@example.test",
  "morgan@example.test",
  "avery@example.test",
  "riley@example.test",
] as const;
const DEMO_NON_PR_TASKS = [
  { kind: "Docs & research", title: "Compare task queue designs" },
  { kind: "Code review", title: "Review retry handling edge cases" },
  { kind: "Docs & research", title: "Draft release documentation" },
  { kind: "Features", title: "Prepare a release checklist" },
  { kind: "Testing", title: "Investigate a flaky integration test" },
] as const;
const DEMO_TASKS = [
  { kind: "Features", title: "Add webhook retry backoff" },
  { kind: "Bug fixes", title: "Fix workspace permissions" },
  { kind: "Features", title: "Improve model routing fallback" },
  { kind: "Features", title: "Add trace search pagination" },
  { kind: "Bug fixes", title: "Resolve duplicate notification delivery" },
  { kind: "Testing", title: "Add API client contract tests" },
  { kind: "Code review", title: "Review cache key isolation" },
  { kind: "Bug fixes", title: "Fix task cancellation handling" },
  { kind: "Docs & research", title: "Document request budget validation" },
] as const;

export function createOutcomeDemo(now: number): OutcomeSession[] {
  return Array.from({ length: 144 }, (_, index) => {
    const group = Math.floor(index / 2);
    const attempt = index % 2;
    const started = now - (3 + group * 12 - attempt * 0.35) * 3_600_000;
    const finished = started + (12 + (group % 19)) * 60_000;
    const occurred = now - (1 + group * 12) * 3_600_000;
    const isLinked = group % 10 !== 9;
    const nonPRTask =
      DEMO_NON_PR_TASKS[Math.floor(group / 10) % DEMO_NON_PR_TASKS.length];
    const task = isLinked ? DEMO_TASKS[group % DEMO_TASKS.length] : nonPRTask;
    const title = task.title;
    const userIndex = (group + Math.floor(group / 9)) % DEMO_USERS.length;
    const repository =
      DEMO_REPOSITORIES[Math.floor(group / 3) % DEMO_REPOSITORIES.length];
    const merged =
      group % 10 < ([8, 5, 3] as const)[group % DEMO_AGENTS.length];
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
      user: DEMO_USERS[userIndex],
      team: OUTCOME_TEAMS[userIndex % OUTCOME_TEAMS.length],
      codingAgent:
        CODING_AGENTS[Math.floor(group / 3) % CODING_AGENTS.length].value,
      taskCategory: task.kind,
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
