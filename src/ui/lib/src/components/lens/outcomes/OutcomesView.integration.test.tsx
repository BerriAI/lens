import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { renderWithLens, stubGateway } from "../../../../tests/lens-test-utils";
import { chooseSelectOption, testQueryClient } from "../../../../tests/test-utils";
import { createOutcomeDemo, filterOutcomeSessions, summarizeOutcomes } from "./outcomesModel";
import { OutcomesView } from "./OutcomesView";

const NOW = Date.parse("2026-11-05T12:00:00Z");
const agent = "release_agent";
const money = (value: number) => value.toLocaleString("en-US", { style: "currency", currency: "USD" });
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

it("shows deduplicated sample outcomes and keeps productivity claims separate from observed spend", () => {
  const gateway = stubGateway();
  const summary = summarizeOutcomes(filtered());
  renderWithLens(<OutcomesView agent={agent} />, { searchParams: `?demo=true&tab=outcomes&agent=${agent}&outcome_scope=agent` });

  expect(
    within(screen.getByRole("group", { name: "Merged pull requests" })).getByText(
      summary.mergedPRs.toLocaleString(),
    ),
  ).toBeVisible();
  expect(
    within(screen.getByRole("group", { name: "Issues resolved" })).getByText(
      summary.closedIssues.toLocaleString(),
    ),
  ).toBeVisible();
  expect(
    within(screen.getByRole("group", { name: "Agent spend" })).getByText(money(summary.totalCostUSD)),
  ).toBeVisible();
  expect(
    within(screen.getByRole("group", { name: "Spend / merged PR" })).getByText(
      money(summary.costPerMergedPR!),
    ),
  ).toBeVisible();
  expect(screen.getByText("Sample data")).toBeVisible();
  expect(screen.getByText(/No estimated hours saved or financial ROI is included/)).toBeVisible();
  expect(screen.getByText("All selected spend ÷ unique merges")).toBeVisible();
  expect(gateway.get).not.toHaveBeenCalled();
});

it("filters the repository and period without changing the selected agent", async () => {
  stubGateway();
  const user = userEvent.setup();
  const onUrlUpdate = vi.fn();
  renderWithLens(<OutcomesView agent={agent} />, {
    searchParams: `?demo=true&tab=outcomes&agent=${agent}&outcome_scope=agent`,
    onUrlUpdate,
  });

  await chooseSelectOption(user, screen.getByRole("combobox", { name: "Repository" }), "acme/console");
  await chooseSelectOption(user, screen.getByRole("combobox", { name: "Outcomes period" }), "Last 7 days");

  const expected = filtered("acme/console", 7);
  const summary = summarizeOutcomes(expected);
  expect(
    within(screen.getByRole("group", { name: "Agent spend" })).getByText(money(summary.totalCostUSD)),
  ).toBeVisible();
  expect(within(screen.getByRole("table", { name: "Session outcomes" })).getAllByRole("row")).toHaveLength(
    Math.min(expected.length, 12) + 1,
  );
  await waitFor(() => expect(onUrlUpdate.mock.lastCall?.[0].queryString).toContain("outcome_days=7"));
  const url = new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString);
  expect(url.get("outcome_repo")).toBe("acme/console");
  expect(url.get("agent")).toBe(agent);
  expect(url.get("demo")).toBe("true");
});

it("opens the recorded merge and closure evidence, while unlinked sessions stay unknown", async () => {
  stubGateway();
  const user = userEvent.setup();
  const onUrlUpdate = vi.fn();
  const linked = filtered().find(
    (session) => session.pullRequest?.status === "merged" && session.issue?.status === "closed",
  )!;
  renderWithLens(<OutcomesView agent={agent} />, {
    searchParams: `?demo=true&tab=outcomes&agent=${agent}&outcome_scope=agent`,
    onUrlUpdate,
  });

  await user.click(screen.getByRole("button", { name: `View evidence for ${linked.id}` }));
  const dialog = await screen.findByRole("dialog", { name: "Session to outcome" });
  expect(within(dialog).getByRole("region", { name: "Link evidence" })).toHaveTextContent(
    "Explicit artifact link",
  );
  expect(within(dialog).getByRole("region", { name: "Outcome timeline" })).toHaveTextContent(
    "Pull request merged",
  );
  expect(within(dialog).getByText(`Issue #${linked.issue!.id} closed`)).toBeVisible();
  expect(within(dialog).getByText("Sample evidence")).toBeVisible();
  await waitFor(() =>
    expect(onUrlUpdate.mock.lastCall?.[0].queryString).toContain(`outcome_session=${linked.id}`),
  );
  await user.click(within(dialog).getByRole("button", { name: "Close", exact: true }));
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  await user.click(screen.getByRole("button", { name: "Review unlinked sessions" }));

  const unlinked = filtered().find((session) => !session.pullRequest && !session.issue)!;
  await user.click(await screen.findByRole("button", { name: `View evidence for ${unlinked.id}` }));
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
  expect(screen.queryByRole("group", { name: "Merged pull requests" })).not.toBeInTheDocument();
  expect(screen.queryByRole("table", { name: "Session outcomes" })).not.toBeInTheDocument();
  expect(gateway.get).not.toHaveBeenCalled();
  expect(gateway.post).not.toHaveBeenCalled();
});
