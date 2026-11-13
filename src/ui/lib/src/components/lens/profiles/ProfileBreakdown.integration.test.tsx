import { screen, within } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { renderWithProviders } from "../../../../tests/test-utils";
import { ProfileBreakdown } from "./ProfileBreakdown";
import { profileColors } from "./profilesData";
import type { ProfileSummary } from "./schemas";

const longName = "Troubleshoot broken features after the latest mobile release rollout for enterprise accounts";

const summary = (profile_id: string, name: string, run_count: number): ProfileSummary => ({
  profile_id,
  name,
  source: "enhanced",
  run_count,
  share: run_count / 8,
  request_count: run_count,
  cost_observations: run_count,
  spend_usd: run_count / 10,
  center: { x: 0, y: 0 },
});

const profiles = [summary("task:long", longName, 4), summary("task:short", "Reset password", 4)];

it("emphasizes the selected profile and keeps long names whole for assistive tech", () => {
  renderWithProviders(
    <ProfileBreakdown profiles={profiles} colors={profileColors(profiles)} selected="task:short" onSelect={vi.fn()} />,
  );

  const list = within(screen.getByRole("list", { name: "Profiles ranked by runs" }));
  expect(list.getByRole("button", { name: new RegExp(longName) })).not.toHaveAttribute("aria-current");
  expect(list.getByRole("button", { name: /Reset password/ })).toHaveAttribute("aria-current", "true");
  expect(list.getAllByRole("button").map((row) => row.textContent)).toEqual([
    expect.stringContaining("Reset password"),
    expect.stringContaining(longName),
  ]);
  expect(list.getAllByRole("button").every((row) => row.textContent?.includes("4 runs"))).toBe(true);
});
