import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { renderWithLens, stubGateway } from "../../../../tests/lens-test-utils";
import { chooseSelectOption, testQueryClient } from "../../../../tests/test-utils";
import { createOutcomeDemo, filterOutcomeSessions, summarizeOutcomes, userOutcomes } from "./outcomesModel";
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

it("ranks people by observed spend and separates linked PR work from other task types", () => {
  const gateway = stubGateway();
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
    expect(cells.slice(1).map((cell) => cell.textContent)).toEqual([
      money(person.totalCostUSD),
      String(person.prSessions),
      String(person.mergedPRs),
      person.prCostPerMerge === null ? "–" : money(person.prCostPerMerge),
      money(person.nonPRSpendUSD),
      percentage(person.nonPRShare),
      person.nonPRTaskKinds
        .slice(0, 2)
        .map((kind) => `${kind.kind} ${percentage(kind.sessionShare)}`)
        .join(", ") || "–",
    ]);
  }
  expect(
    screen.getByText(
      `${money(summary.totalCostUSD)} across ${summary.sessions} sessions · ${summary.mergedPRs} merged PRs · ${summary.closedIssues} closed issues`,
    ),
  ).toBeVisible();
  expect(screen.getByText("Sample data")).toBeVisible();
  expect(screen.getByText(/Cost per merge uses their spend and unique merged PRs/)).toBeVisible();
  expect(screen.getByText(/association, not measured productivity lift/)).toBeVisible();
  expect(screen.queryByRole("table", { name: "Session outcomes" })).not.toBeInTheDocument();
  expect(gateway.get).not.toHaveBeenCalled();
});

it("scopes the report and session drilldown while preserving the agent and existing repository filter", async () => {
  stubGateway();
  const user = userEvent.setup();
  const onUrlUpdate = vi.fn();
  renderWithLens(<OutcomesView agent={agent} />, {
    searchParams: `?demo=true&tab=outcomes&agent=${agent}&outcome_repo=acme%2Fconsole`,
    onUrlUpdate,
  });

  await chooseSelectOption(user, screen.getByRole("combobox", { name: "Outcomes scope" }), agent);
  await chooseSelectOption(user, screen.getByRole("combobox", { name: "Outcomes period" }), "Last 7 days");
  const expected = filtered("acme/console", 7);
  const people = userOutcomes(expected);
  const table = screen.getByRole("table", { name: "Usage by person" });
  expect(within(table).getAllByRole("button", { name: /^View sessions for / })).toHaveLength(people.length);
  await user.click(within(table).getByRole("button", { name: `View sessions for ${people[0].user}` }));
  const dialog = await screen.findByRole("dialog", { name: `Sessions for ${people[0].user}` });
  expect(within(dialog).getByRole("table", { name: "Session outcomes" })).toBeVisible();
  expect(within(dialog).getAllByRole("button", { name: /^Open trace for / })).toHaveLength(
    people[0].sessions,
  );
  await user.click(within(dialog).getByRole("button", { name: "Close", exact: true }));
  await waitFor(() => expect(onUrlUpdate.mock.lastCall?.[0].queryString).toContain("outcome_days=7"));
  const url = new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString);
  expect(url.get("outcome_repo")).toBe("acme/console");
  expect(url.get("outcome_scope")).toBe("agent");
  expect(url.get("agent")).toBe(agent);
  expect(url.get("demo")).toBe("true");

  await user.click(screen.getByRole("button", { name: "Reset filters" }));
  await waitFor(() =>
    expect(new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString).has("outcome_repo")).toBe(false),
  );
  expect(screen.getByRole("combobox", { name: "Outcomes scope" })).toHaveTextContent(agent);
  expect(screen.getByRole("combobox", { name: "Outcomes period" })).toHaveTextContent("Last 7 days");
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

  expect(screen.getByText("Explore the outcomes dashboard")).toBeVisible();
  expect(screen.getByText(/Live outcome ingestion is not connected in this preview/)).toBeVisible();
  expect(screen.getByRole("button", { name: "Explore sample dashboard" })).toBeVisible();
  expect(screen.queryByRole("table", { name: "Usage by person" })).not.toBeInTheDocument();
  expect(screen.queryByRole("table", { name: "Session outcomes" })).not.toBeInTheDocument();
  expect(gateway.get).not.toHaveBeenCalled();
  expect(gateway.post).not.toHaveBeenCalled();
});
