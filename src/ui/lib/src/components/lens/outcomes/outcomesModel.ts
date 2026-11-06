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
  nonPRTaskKinds: { kind: string; sessions: number; sessionShare: number }[];
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
      const totalCostUSD = summarizeOutcomes(owned).totalCostUSD;
      const pr = summarizeOutcomes(prSessions);
      const nonPRSpendUSD = summarizeOutcomes(nonPRSessions).totalCostUSD;
      const nonPRTaskKinds = [
        ...new Set(nonPRSessions.map((session) => session.taskCategory)),
      ]
        .map((kind) => {
          const count = nonPRSessions.filter(
            (session) => session.taskCategory === kind,
          ).length;
          return {
            kind,
            sessions: count,
            sessionShare: count / nonPRSessions.length,
          };
        })
        .sort(
          (left, right) =>
            right.sessions - left.sessions ||
            left.kind.localeCompare(right.kind),
        );
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
      };
    })
    .sort(
      (left, right) =>
        right.totalCostUSD - left.totalCostUSD ||
        left.user.localeCompare(right.user),
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
  { kind: "Research", title: "Compare task queue designs" },
  { kind: "Code review", title: "Review retry handling edge cases" },
  { kind: "Documentation", title: "Draft release documentation" },
  { kind: "Workflow automation", title: "Prepare a release checklist" },
  { kind: "Debugging", title: "Investigate a flaky integration test" },
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
    const isLinked = group % 10 !== 9;
    const nonPRTask =
      DEMO_NON_PR_TASKS[Math.floor(group / 10) % DEMO_NON_PR_TASKS.length];
    const title = isLinked
      ? DEMO_TASKS[group % DEMO_TASKS.length]
      : nonPRTask.title;
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
      user: DEMO_USERS[(group + Math.floor(group / 9)) % DEMO_USERS.length],
      taskCategory: isLinked ? "Coding" : nonPRTask.kind,
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
