import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import { renderWithLens } from "../../../../tests/lens-test-utils";
import { testQueryClient } from "../../../../tests/test-utils";
import { createLensDemo } from "../data/demo/createLensDemo";
import { createOutcomeDemo } from "./outcomesModel";
import { outcomeTraceId } from "./outcomeTrace";
import { TraceDeliveryOutcome } from "./TraceDeliveryOutcome";

const NOW = Date.parse("2026-11-05T12:00:00Z");

beforeEach(() => {
  testQueryClient.clear();
  vi.spyOn(Date, "now").mockReturnValue(NOW);
});

afterEach(() => vi.restoreAllMocks());

it("shows the linked merge and issue closure on a demo trace and returns to its aggregate scope", async () => {
  const session = createOutcomeDemo(NOW)[0];
  const traceId = outcomeTraceId(session);
  const trace = await createLensDemo(NOW).traces.trace(traceId);
  const user = userEvent.setup();
  const onUrlUpdate = vi.fn();
  renderWithLens(
    <TraceDeliveryOutcome summary={trace.summary} live={false} />,
    {
      searchParams: `?demo=true&tab=traces&agent=${session.agent}&trace=${traceId}&fullscreen=true&outcome_repo=${session.repository}&outcome_team=${encodeURIComponent(session.team)}&outcome_coding_agent=${session.codingAgent}&outcome_from=2026-10-26&outcome_to=2026-11-05`,
      onUrlUpdate,
    },
  );

  const outcomes = screen.getByRole("region", { name: "Delivery outcomes" });
  expect(within(outcomes).getByText("PR #480 merged")).toBeVisible();
  expect(within(outcomes).getByText("Issue #210 closed")).toBeVisible();
  expect(within(outcomes).getByText("Sample data")).toBeVisible();
  expect(within(outcomes).getByText(session.title)).toBeVisible();
  await user.click(
    within(outcomes).getByRole("button", { name: "View coding outcomes" }),
  );
  await waitFor(() => expect(onUrlUpdate).toHaveBeenCalled());
  const url = new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString);
  expect(url.get("tab")).toBe("outcomes");
  expect(url.get("agent")).toBe(session.agent);
  expect(url.get("outcome_repo")).toBe(session.repository);
  expect(url.get("outcome_team")).toBe(session.team);
  expect(url.get("outcome_coding_agent")).toBe(session.codingAgent);
  expect(url.get("outcome_from")).toBe("2026-10-26");
  expect(url.get("outcome_to")).toBe("2026-11-05");
  expect(url.has("trace")).toBe(false);
  expect(url.has("fullscreen")).toBe(false);
});

it("never presents sample artifact outcomes on a live trace", async () => {
  const session = createOutcomeDemo(NOW)[0];
  const trace = await createLensDemo(NOW).traces.trace(outcomeTraceId(session));
  renderWithLens(<TraceDeliveryOutcome summary={trace.summary} live />, {
    searchParams: `?tab=traces&agent=${session.agent}&trace=${trace.summary.trace_id}`,
  });

  expect(
    screen.queryByRole("region", { name: "Delivery outcomes" }),
  ).not.toBeInTheDocument();
  expect(screen.queryByText("PR #480 merged")).not.toBeInTheDocument();
  expect(screen.queryByText("Issue #210 closed")).not.toBeInTheDocument();
});
