import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { renderWithLens, stubGateway } from "../../../../tests/lens-test-utils";
import { chooseSelectOption, testQueryClient } from "../../../../tests/test-utils";
import { CODING_AGENTS, createOutcomeDemo, filterOutcomeSessions, summarizeOutcomes, taskBreakdown, userOutcomes } from "./outcomesModel";
import { OutcomesView } from "./OutcomesView";

const NOW = Date.parse("2026-11-05T12:00:00Z");
const agent = "release_agent";
const money = (value: number) => value.toLocaleString("en-US", { style: "currency", currency: "USD" });
const percentage = (value: number) => `${Math.round(value * 100)}%`;
const filtered = (repository = "all", days = 30) =>
  filterOutcomeSessions(createOutcomeDemo(NOW), { agent, repository, days, status: "all" }, NOW);

beforeEach(() => {
  testQueryClient.clear();
  vi.spyOn(Date, "now").mockReturnValue(NOW);
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it("shows filtered task activity and each person’s spend, merged PRs and closed issues", async () => {
  const gateway = stubGateway();
  const user = userEvent.setup();
  const sessions = filtered();
  const summary = summarizeOutcomes(sessions);
  const people = userOutcomes(sessions);
  renderWithLens(<OutcomesView agent={agent} />, {
    searchParams: `?demo=true&tab=outcomes&agent=${agent}&outcome_scope=agent`,
  });

  const table = screen.getByRole("table", { name: "Usage by person" });
  expect(
    within(table)
      .getAllByRole("button", { name: /^View sessions for / })
      .map((button) => button.getAttribute("aria-label")),
  ).toEqual(people.map((person) => `View sessions for ${person.user}`));
  for (const person of people) {
    const row = within(table).getByRole("row", { name: person.user });
    const cells = within(row).getAllByRole("cell");
    expect(cells.slice(1, 6).map((cell) => cell.textContent)).toEqual([
      money(person.totalCostUSD),
      String(person.sessions),
      String(person.mergedPRs),
      String(person.closedIssues),
      person.prCostPerMerge === null ? "–" : money(person.prCostPerMerge),
    ]);
    const mix = person.taskKinds.map((kind) => `${kind.kind} ${percentage(kind.sessionShare)}`).join(", ");
    expect(within(cells[6]).getByRole("img")).toHaveAccessibleName(`${person.user} task activity: ${mix}`);
    expect(cells[6]).toHaveTextContent(person.taskKinds.slice(0, 2).map((kind) => `${kind.kind} ${percentage(kind.sessionShare)}`).join(", "));
  }
  for (const [label, value] of [
    ["Sessions", String(summary.sessions)],
    ["Merged pull requests", String(summary.mergedPRs)],
    ["Closed issues", String(summary.closedIssues)],
    ["Agent spend", money(summary.totalCostUSD)],
  ]) {
    expect(within(screen.getByRole("group", { name: label })).getByText(value, { exact: true })).toBeVisible();
  }
  const activity = screen.getByRole("region", { name: "Task activity" });
  const categories = taskBreakdown(sessions);
  expect(within(activity).getByRole("img")).toHaveAccessibleName(
    `Task activity stacked bar chart: ${summary.sessions.toLocaleString()} sessions. ${categories
      .map((kind) => `${kind.kind}: ${kind.sessions.toLocaleString()} sessions, ${percentage(kind.sessionShare)}`)
      .join("; ")}`,
  );
  expect(within(activity).getByRole("list", { name: "Task categories" })).toHaveTextContent(
    categories.map((kind) => `${kind.kind}${kind.sessions.toLocaleString()} sessions${percentage(kind.sessionShare)}`).join(""),
  );
  await user.click(within(activity).getByText("Daily data"));
  const daily = within(activity).getByRole("table", { name: "Task activity daily data" });
  const dailyRows = within(daily).getAllByRole("row").slice(1).map((row) => within(row).getAllByRole("cell"));
  const columnTotal = (column: number) => dailyRows.reduce((sum, cells) => sum + Number(cells[column].textContent?.replaceAll(",", "")), 0);
  expect(columnTotal(categories.length + 1)).toBe(summary.sessions);
  categories.forEach((kind, index) => expect(columnTotal(index + 1)).toBe(kind.sessions));
  expect(screen.getByText("Sample data")).toBeVisible();
  expect(screen.getByText(/Task activity includes all sessions/)).toBeVisible();
  expect(screen.queryByRole("table", { name: "Session outcomes" })).not.toBeInTheDocument();
  expect(gateway.get).not.toHaveBeenCalled();
});

it("intersects team, coding agent, repository and period filters through a person’s sessions", async () => {
  stubGateway();
  const user = userEvent.setup();
  const onUrlUpdate = vi.fn();
  const target = filtered("acme/console", 7)[0];
  renderWithLens(<OutcomesView agent={agent} />, {
    searchParams: `?demo=true&tab=outcomes&agent=${agent}&outcome_scope=agent&outcome_repo=acme%2Fconsole`,
    onUrlUpdate,
  });

  await chooseSelectOption(user, screen.getByRole("combobox", { name: "Team", exact: true }), target.team);
  await chooseSelectOption(user, screen.getByRole("combobox", { name: "Coding agent", exact: true }),
    CODING_AGENTS.find((item) => item.value === target.codingAgent)!.label);
  await chooseSelectOption(user, screen.getByRole("combobox", { name: "Time range" }), "Last 7 days");
  const expected = filterOutcomeSessions(createOutcomeDemo(NOW), {
    agent, repository: "acme/console", days: 7, status: "all", team: target.team, codingAgent: target.codingAgent,
  }, NOW);
  const people = userOutcomes(expected);
  const table = screen.getByRole("table", { name: "Usage by person" });
  expect(within(table).getAllByRole("button", { name: /^View sessions for / })).toHaveLength(people.length);
  const activity = screen.getByRole("region", { name: "Task activity" });
  expect(within(activity).getByRole("img")).toHaveAccessibleName(expect.stringContaining(`${expected.length} sessions.`));
  await user.click(within(table).getByRole("button", { name: `View sessions for ${people[0].user}` }));
  const dialog = await screen.findByRole("dialog", { name: `Sessions for ${people[0].user}` });
  expect(within(dialog).getByRole("table", { name: "Session outcomes" })).toBeVisible();
  expect(within(dialog).getAllByRole("button", { name: /^Open trace for / }).map((button) => button.getAttribute("aria-label"))).toEqual(
    expected.filter((session) => session.user === people[0].user).map((session) => `Open trace for ${session.id}`),
  );
  await user.click(within(dialog).getByRole("button", { name: "Close", exact: true }));
  await waitFor(() => expect(onUrlUpdate.mock.lastCall?.[0].queryString).toContain("outcome_days=7"));
  const url = new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString);
  expect(url.get("outcome_repo")).toBe("acme/console");
  expect(url.get("outcome_team")).toBe(target.team);
  expect(url.get("outcome_coding_agent")).toBe(target.codingAgent);
  expect(url.get("outcome_scope")).toBe("agent");
  expect(url.get("agent")).toBe(agent);
  expect(url.get("demo")).toBe("true");

  await user.click(screen.getByRole("button", { name: "Reset filters" }));
  await waitFor(() => expect(new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString).has("outcome_repo")).toBe(false));
  expect(screen.getByRole("combobox", { name: "Team", exact: true })).toHaveTextContent("All teams");
  expect(screen.getByRole("combobox", { name: "Coding agent", exact: true })).toHaveTextContent("All coding agents");
  expect(screen.getByRole("combobox", { name: "Time range" })).toHaveTextContent("Last 7 days");
});

it("shares an inclusive custom range and clears its dates when a preset is selected", async () => {
  stubGateway();
  const user = userEvent.setup();
  const onUrlUpdate = vi.fn();
  const target = createOutcomeDemo(NOW)[0];
  const filters = { agent, repository: "all", days: 30, status: "all", team: target.team, codingAgent: target.codingAgent, from: "2026-10-26", to: "2026-11-04" } as const;
  const view = renderWithLens(<OutcomesView agent={agent} />, {
    searchParams: `?demo=true&tab=outcomes&agent=${agent}&outcome_scope=agent&outcome_team=${encodeURIComponent(target.team)}&outcome_coding_agent=${target.codingAgent}`,
    onUrlUpdate,
  });
  await chooseSelectOption(user, screen.getByRole("combobox", { name: "Time range" }), "Custom range");
  const range = await screen.findByRole("dialog", { name: "Custom date range" });
  fireEvent.change(within(range).getByLabelText("From date"), { target: { value: filters.from } });
  fireEvent.change(within(range).getByLabelText("To date"), { target: { value: filters.to } });
  await user.click(within(range).getByRole("button", { name: "Apply range" }));
  await waitFor(() => expect(onUrlUpdate.mock.lastCall?.[0].queryString).toContain(`outcome_from=${filters.from}`));
  const url = new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString);
  expect(url.get("outcome_to")).toBe(filters.to);
  expect(url.get("outcome_team")).toBe(target.team);
  expect(url.get("outcome_coding_agent")).toBe(target.codingAgent);
  expect(url.get("agent")).toBe(agent);
  view.unmount();

  renderWithLens(<OutcomesView agent={agent} />, { searchParams: url, onUrlUpdate });
  const expected = filterOutcomeSessions(createOutcomeDemo(NOW), filters, NOW);
  const people = userOutcomes(expected);
  const table = screen.getByRole("table", { name: "Usage by person" });
  expect(within(table).getAllByRole("button", { name: /^View sessions for / }).map((button) => button.getAttribute("aria-label"))).toEqual(
    people.map((person) => `View sessions for ${person.user}`),
  );
  expect(within(screen.getByRole("group", { name: "Sessions" })).getByText(String(expected.length), { exact: true })).toBeVisible();
  const activity = screen.getByRole("region", { name: "Task activity" });
  expect(within(activity).getByRole("img")).toHaveAccessibleName(expect.stringContaining(`${expected.length} sessions.`));
  await user.click(screen.getByRole("button", { name: `View sessions for ${people[0].user}` }));
  const dialog = await screen.findByRole("dialog", { name: `Sessions for ${people[0].user}` });
  expect(within(dialog).getAllByRole("button", { name: /^Open trace for / })).toHaveLength(people[0].sessions);
  await user.click(within(dialog).getByRole("button", { name: "Close", exact: true }));
  await chooseSelectOption(user, screen.getByRole("combobox", { name: "Time range" }), "Last 90 days");
  await waitFor(() => expect(onUrlUpdate.mock.lastCall?.[0].queryString).toContain("outcome_days=90"));
  const preset = new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString);
  expect(preset.has("outcome_from")).toBe(false);
  expect(preset.has("outcome_to")).toBe(false);
  expect(preset.get("outcome_team")).toBe(target.team);
  expect(preset.get("outcome_coding_agent")).toBe(target.codingAgent);
});

it("opens recorded evidence from a person's sessions and supports direct links to unknown outcomes", async () => {
  stubGateway();
  const user = userEvent.setup();
  const onUrlUpdate = vi.fn();
  const linked = filtered().find(
    (session) => session.pullRequest?.status === "merged" && session.issue?.status === "closed",
  )!;
  const view = renderWithLens(<OutcomesView agent={agent} />, {
    searchParams: `?demo=true&tab=outcomes&agent=${agent}&outcome_scope=agent`,
    onUrlUpdate,
  });

  await user.click(screen.getByRole("button", { name: `View sessions for ${linked.user}` }));
  const sessions = await screen.findByRole("dialog", { name: `Sessions for ${linked.user}` });
  await user.click(within(sessions).getByRole("button", { name: `View evidence for ${linked.id}` }));
  const dialog = await screen.findByRole("dialog", { name: "Session to outcome" });
  expect(within(dialog).getByRole("region", { name: "Link evidence" })).toHaveTextContent(
    "Explicit artifact link",
  );
  expect(within(dialog).getByRole("region", { name: "Outcome timeline" })).toHaveTextContent(
    "Pull request merged",
  );
  expect(within(dialog).getByText(`Issue #${linked.issue!.id} closed`)).toBeVisible();
  expect(within(dialog).getByText("Sample evidence")).toBeVisible();
  expect(within(dialog).getByRole("button", { name: "Open trace", exact: true })).toBeVisible();
  await waitFor(() =>
    expect(onUrlUpdate.mock.lastCall?.[0].queryString).toContain(`outcome_session=${linked.id}`),
  );
  await user.click(within(dialog).getByRole("button", { name: "Close", exact: true }));
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  view.unmount();

  const unlinked = filtered().find((session) => !session.pullRequest && !session.issue)!;
  renderWithLens(<OutcomesView agent={agent} />, {
    searchParams: `?demo=true&tab=outcomes&agent=${agent}&outcome_scope=agent&outcome_session=${unlinked.id}`,
  });
  const unknownDialog = await screen.findByRole("dialog", { name: "Session to outcome" });
  expect(within(unknownDialog).getByRole("region", { name: "Link evidence" })).toHaveTextContent(
    "Its delivery outcome is unknown",
  );
  expect(within(unknownDialog).queryByText("Explicit artifact link")).not.toBeInTheDocument();
  expect(within(unknownDialog).queryByText("Pull request merged")).not.toBeInTheDocument();
});

it("shows a preview entry point on a live route without displaying sample aggregates", () => {
  const gateway = stubGateway();
  renderWithLens(<OutcomesView agent={agent} />, { searchParams: `?tab=outcomes&agent=${agent}` });

  expect(screen.getByText("Explore coding agent outcomes")).toBeVisible();
  expect(screen.getByText(/Live outcome ingestion is not connected in this preview/)).toBeVisible();
  expect(screen.getByRole("button", { name: "Explore sample dashboard" })).toBeVisible();
  expect(screen.queryByRole("table", { name: "Usage by person" })).not.toBeInTheDocument();
  expect(screen.queryByRole("table", { name: "Session outcomes" })).not.toBeInTheDocument();
  expect(gateway.get).not.toHaveBeenCalled();
  expect(gateway.post).not.toHaveBeenCalled();
});
