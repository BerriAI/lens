import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { renderWithProviders } from "../../../../tests/test-utils";
import { ProfileMap } from "./ProfileMap";
import { profileColors } from "./profilesData";
import type { ProfilesOverview, ProfileSummary } from "./schemas";

const summary = (profile_id: string, name: string, run_count: number, source: ProfileSummary["source"]): ProfileSummary => ({
  profile_id,
  name,
  source,
  run_count,
  share: run_count / 10,
  request_count: run_count,
  cost_observations: run_count,
  spend_usd: 1,
  center: { x: 0, y: 0 },
});

const profiles = [
  summary("task:a", "Answer billing questions", 3, "enhanced"),
  summary("task:b", "Reset a forgotten password", 3, "enhanced"),
  summary("unknown", "Unknown", 2, "unknown"),
  summary("recorded:Refunds", "Refunds", 2, "recorded"),
];

const overview = (points: ProfilesOverview["points"]): ProfilesOverview => ({
  window: { agent: "assistant", start_ms: 0, end_ms: 1 },
  coverage: { runs_listed: 10, runs_resolved: 10, truncated: false },
  totals: { runs: 10, requests: 10, matched_requests: 10, cost_observations: 10, spend_usd: 1, other_agent_requests: 0, shared_requests: 0 },
  profiles,
  points,
  taxonomy: null,
});

const discovered = overview([
  { trace_id: "a1", profile_id: "task:a", x: 0, y: 0, placement: "discovery" },
  { trace_id: "a2", profile_id: "task:a", x: 1, y: 0, placement: "discovery" },
  { trace_id: "a3", profile_id: "task:a", x: 0, y: 1, placement: "discovery" },
  { trace_id: "b1", profile_id: "task:b", x: 100, y: 100, placement: "discovery" },
  { trace_id: "b2", profile_id: "task:b", x: 101, y: 100, placement: "discovery" },
  { trace_id: "b3", profile_id: "task:b", x: 60, y: 60, placement: "categorical" },
  { trace_id: "u1", profile_id: "unknown", x: 50, y: 50, placement: "discovery" },
  { trace_id: "r1", profile_id: "recorded:Refunds", x: 20, y: 20, placement: "categorical" },
]);

function render(data: ProfilesOverview, selected: string | null = null, onOpenDetails = vi.fn()) {
  return renderWithProviders(
    <ProfileMap overview={data} colors={profileColors(data.profiles)} selected={selected} onOpenDetails={onOpenDetails} />,
  );
}

beforeEach(() => {
  vi.stubGlobal(
    "ResizeObserver",
    class implements ResizeObserver {
      constructor(private readonly callback: ResizeObserverCallback) {}
      observe(target: Element) {
        const size = [{ inlineSize: 600, blockSize: 360 }];
        this.callback(
          [
            {
              target,
              contentRect: new DOMRect(0, 0, 600, 360),
              borderBoxSize: size,
              contentBoxSize: size,
              devicePixelContentBoxSize: size,
            },
          ],
          this,
        );
      }
      unobserve() {}
      disconnect() {}
    },
  );
});
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it("draws only discovery positions of real groups and counts every other run beside the map", async () => {
  render(discovered);

  expect(await screen.findByRole("group", { name: "Similarity map of 5 runs in 2 groups" })).toBeVisible();
  expect(screen.getByText(/5 of 10 runs on the map/)).toBeVisible();
  const outside = within(screen.getByRole("region", { name: "Runs not on the map" }));
  expect(outside.getByRole("button", { name: /Unknown/ })).toHaveTextContent("2");
  expect(outside.getByRole("button", { name: /Refunds/ })).toHaveTextContent("2");
  expect(outside.getByRole("button", { name: /Reset a forgotten password/ })).toHaveTextContent("1");
  const map = within(screen.getByRole("group", { name: "Similarity map of 5 runs in 2 groups" }));
  expect(map.getAllByRole("button").map((label) => label.getAttribute("aria-label"))).toEqual([
    "Answer billing questions, 3 runs on the map",
    "Reset a forgotten password, 2 runs on the map",
  ]);
});

it("focuses a group from its label by pointer or keyboard and opens details only on request", async () => {
  const user = userEvent.setup();
  const onOpenDetails = vi.fn();
  render(discovered, null, onOpenDetails);
  const billing = await screen.findByRole("button", { name: "Answer billing questions, 3 runs on the map" });

  await user.click(billing);

  expect(billing).toHaveAttribute("aria-pressed", "true");
  expect(onOpenDetails).not.toHaveBeenCalled();

  screen.getByRole("button", { name: "Reset a forgotten password, 2 runs on the map" }).focus();
  await user.keyboard("{Enter}");

  expect(screen.getByRole("button", { name: "Reset a forgotten password, 2 runs on the map" })).toHaveAttribute("aria-pressed", "true");
  expect(billing).toHaveAttribute("aria-pressed", "false");
  await user.click(screen.getByRole("button", { name: "View details" }));
  expect(onOpenDetails).toHaveBeenCalledWith("task:b");
});

it("zooms to the focused group and back to every group", async () => {
  const user = userEvent.setup();
  render(discovered);

  expect(screen.queryByRole("button", { name: "Zoom to selection" })).not.toBeInTheDocument();
  await user.click(await screen.findByRole("button", { name: "Answer billing questions, 3 runs on the map" }));
  expect(screen.getByRole("button", { name: "Reset a forgotten password, 2 runs on the map" })).toBeVisible();

  await user.click(screen.getByRole("button", { name: "Zoom to selection" }));

  expect(screen.getByRole("button", { name: "Show all groups" })).toHaveAttribute("aria-pressed", "true");
  expect(screen.queryByRole("button", { name: "Reset a forgotten password, 2 runs on the map" })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Answer billing questions, 3 runs on the map" })).toBeVisible();

  await user.click(screen.getByRole("button", { name: "Show all groups" }));

  expect(screen.getByRole("button", { name: "Reset a forgotten password, 2 runs on the map" })).toBeVisible();
});

it("explains where positions come from when no run has one", async () => {
  render(overview([{ trace_id: "r1", profile_id: "recorded:Refunds", x: 20, y: 20, placement: "categorical" }]));

  expect(await screen.findByRole("status")).toHaveTextContent("No similarity positions in this range");
  expect(screen.queryByRole("group", { name: /Similarity map/ })).not.toBeInTheDocument();
});

it("draws runs from different teams that share a trace id without colliding", async () => {
  const errors = vi.spyOn(console, "error").mockImplementation(() => {});
  render(
    overview([
      { trace_id: "same-trace", profile_id: "task:a", x: 0, y: 0, placement: "discovery" },
      { trace_id: "same-trace", profile_id: "task:a", x: 5, y: 5, placement: "discovery" },
    ]),
  );

  expect(await screen.findByRole("group", { name: "Similarity map of 2 runs in 1 group" })).toBeVisible();
  expect(errors.mock.calls.flat().join(" ")).not.toContain("same key");
});

it("focuses a group from the side list and clears the focus when chosen again", async () => {
  const user = userEvent.setup();
  render(discovered);
  const groups = within(await screen.findByRole("region", { name: "Groups on the map" }));
  const entry = groups.getByRole("button", { name: /Reset a forgotten password/ });

  await user.click(entry);

  expect(entry).toHaveAttribute("aria-pressed", "true");
  expect(screen.getByRole("button", { name: "Reset a forgotten password, 2 runs on the map" })).toHaveAttribute("aria-pressed", "true");
  expect(screen.getByRole("button", { name: "Answer billing questions, 3 runs on the map" })).toHaveAttribute("opacity", "0.45");

  await user.click(entry);
  await user.unhover(entry);

  expect(entry).toHaveAttribute("aria-pressed", "false");
  expect(screen.queryByRole("button", { name: "Zoom to selection" })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Answer billing questions, 3 runs on the map" })).toHaveAttribute("opacity", "1");
});

it("previews a group's emphasis while hovering its label without choosing it", async () => {
  const user = userEvent.setup();
  render(discovered);
  const billing = await screen.findByRole("button", { name: "Answer billing questions, 3 runs on the map" });
  const password = screen.getByRole("button", { name: "Reset a forgotten password, 2 runs on the map" });

  await user.hover(billing);

  expect(password).toHaveAttribute("opacity", "0.45");
  expect(billing).toHaveAttribute("aria-pressed", "false");

  await user.unhover(billing);

  expect(password).toHaveAttribute("opacity", "1");
});

it("opens details for a run group that has no position", async () => {
  const user = userEvent.setup();
  const onOpenDetails = vi.fn();
  render(discovered, null, onOpenDetails);

  const outside = within(await screen.findByRole("region", { name: "Runs not on the map" }));
  await user.click(outside.getByRole("button", { name: /View details for Refunds/ }));

  expect(onOpenDetails).toHaveBeenCalledWith("recorded:Refunds");
});

it("starts a newly chosen group in the full view after an earlier zoom was cleared", async () => {
  const user = userEvent.setup();
  render(discovered);
  const billing = await screen.findByRole("button", { name: "Answer billing questions, 3 runs on the map" });
  await user.click(billing);
  await user.click(screen.getByRole("button", { name: "Zoom to selection" }));
  expect(screen.queryByRole("button", { name: "Reset a forgotten password, 2 runs on the map" })).not.toBeInTheDocument();

  await user.click(billing);
  const groups = within(screen.getByRole("region", { name: "Groups on the map" }));
  await user.click(groups.getByRole("button", { name: /Reset a forgotten password/ }));

  expect(screen.getByRole("button", { name: "Zoom to selection" })).toHaveAttribute("aria-pressed", "false");
  expect(screen.getByRole("button", { name: "Answer billing questions, 3 runs on the map" })).toBeVisible();
  expect(screen.getByRole("button", { name: "Reset a forgotten password, 2 runs on the map" })).toHaveAttribute("aria-pressed", "true");
});
