import { fireEvent, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { renderWithLens, stubGateway } from "../../../../tests/lens-test-utils";
import { testQueryClient } from "../../../../tests/test-utils";
import type { LensAgents } from "../agents/AgentScoped";
import { ProfilesView } from "./ProfilesView";

const agents: LensAgents = {
  agent: "assistant",
  select: vi.fn(),
  list: {
    agents: [
      { name: "assistant", runs: 20, failed_runs: 0, last_seen: new Date().toISOString(), frameworks: [] },
    ],
    isLoading: false,
    error: null,
  },
};

beforeEach(() => testQueryClient.clear());
afterEach(() => vi.restoreAllMocks());

it("shows full aggregate counts and shares, including unclassified, through the live query client", async () => {
  const proxy = stubGateway();
  proxy.post.mockResolvedValue({
    data: [
      { category: "Coding", runs: "10" },
      { category: "Workflows", runs: "6" },
      { category: "Office automation", runs: "3" },
      { category: "Unclassified", runs: "1" },
    ],
  });
  const onUrlUpdate = vi.fn();
  renderWithLens(<ProfilesView agents={agents} />, {
    searchParams: "?tab=profiles&agent=assistant",
    onUrlUpdate,
  });
  expect(await screen.findByRole("meter", { name: "Coding" })).toHaveAttribute(
    "aria-valuetext",
    "10 of 20 runs, 50%",
  );
  expect(screen.getByRole("meter", { name: "Workflows" })).toHaveAttribute(
    "aria-valuetext",
    "6 of 20 runs, 30%",
  );
  expect(screen.getByRole("meter", { name: "Unclassified" })).toHaveAttribute(
    "aria-valuetext",
    "1 of 20 runs, 5%",
  );
  expect(screen.getByRole("img", { name: /Usage pie chart/ })).toBeVisible();
  expect(proxy.post).toHaveBeenCalledWith(
    "/v1/traces/query",
    expect.objectContaining({
      body: expect.objectContaining({ sql: expect.stringContaining("'assistant'") }),
    }),
  );

  fireEvent.change(screen.getByRole("combobox", { name: "Profile agent" }), { target: { value: "" } });
  await waitFor(() => expect(onUrlUpdate).toHaveBeenCalled());
  await waitFor(() => expect(proxy.post).toHaveBeenCalledTimes(2));
});

it.each(["Coding", "Unclassified"])("opens investigation setup with the selected %s profile, agent and lookback", async (category) => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  proxy.post.mockResolvedValue({ data: [{ category, runs: 10 }] });
  const onUrlUpdate = vi.fn();
  renderWithLens(<ProfilesView agents={agents} />, { searchParams: "?tab=profiles&agent=assistant&hours=168", onUrlUpdate });
  expect(await screen.findByRole("button", { name: "Investigate profile" })).toBeDisabled();
  await user.click(await screen.findByRole("button", { name: new RegExp(`^${category}`) }));
  await user.click(screen.getByRole("button", { name: "Investigate profile" }));
  await waitFor(() => expect(onUrlUpdate.mock.lastCall?.[0].queryString).toContain("dialog=new"));
  const url = new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString);
  expect(url.get("profile")).toBe(category);
  expect(url.get("agent")).toBe("assistant");
  expect(url.get("hours")).toBe("168");
  expect(url.get("tab")).toBe("findings");
});

it("shows an empty state when the selected range has no runs", async () => {
  const proxy = stubGateway();
  proxy.post.mockResolvedValue({ data: [] });
  renderWithLens(<ProfilesView agents={agents} />);
  expect(await screen.findByRole("heading", { name: "No runs in this time range" })).toBeVisible();
  expect(screen.queryByRole("figure")).not.toBeInTheDocument();
});

it("lets a failed aggregate be retried without displaying fabricated counts", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  proxy.post.mockRejectedValueOnce(new Error("Query unavailable"));
  proxy.post.mockResolvedValue({ data: [{ category: "Unclassified", runs: 2 }] });
  renderWithLens(<ProfilesView agents={agents} />);
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not load usage profiles");
  expect(screen.queryByRole("meter")).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Refresh usage profiles" }));
  expect(await screen.findByRole("meter", { name: "Unclassified" })).toHaveAttribute(
    "aria-valuetext",
    "2 of 2 runs, 100%",
  );
  expect(screen.getByText(/Record task_category or scenario/)).toBeVisible();
});

it("advances the full range on refresh and hides old counts if the refreshed query fails", async () => {
  const user = userEvent.setup();
  const now = Date.now();
  const clock = vi.spyOn(Date, "now").mockReturnValue(now);
  const proxy = stubGateway();
  proxy.post.mockResolvedValueOnce({ data: [{ category: "Coding", runs: 12 }] });
  proxy.post.mockRejectedValueOnce(new Error("Query unavailable"));
  renderWithLens(<ProfilesView agents={agents} />);
  expect(await screen.findByRole("meter", { name: "Coding" })).toHaveAttribute("aria-valuemax", "12");
  clock.mockReturnValue(now + 60_000);
  await user.click(screen.getByRole("button", { name: "Refresh usage profiles" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not load usage profiles");
  expect(screen.queryByRole("meter")).not.toBeInTheDocument();
  expect(proxy.post.mock.lastCall?.[1].body).toEqual({
    sql: expect.stringContaining(`fromUnixTimestamp64Milli(${now + 60_000})`),
  });
});

it("preserves a shared historical window when opening and refreshing its usage profile", async () => {
  const user = userEvent.setup();
  const end = Date.now() - 7 * 86_400_000;
  const start = end - 3_600_000;
  const proxy = stubGateway();
  proxy.post.mockResolvedValue({ data: [{ category: "Coding", runs: 1 }] });
  renderWithLens(<ProfilesView agents={agents} />, { searchParams: `?tab=profiles&from=${start}&to=${end}` });
  expect(await screen.findByRole("meter", { name: "Coding" })).toBeVisible();
  await user.click(screen.getByRole("button", { name: /^Coding/ }));
  expect(screen.getByRole("button", { name: "Investigate profile" })).toBeDisabled();
  expect(screen.getByText("Choose a relative time range to investigate this profile. Investigations review recent runs.")).toBeVisible();
  expect(proxy.post.mock.lastCall?.[1].body).toEqual({ sql: expect.stringContaining(`fromUnixTimestamp64Milli(${start})`) });
  expect(proxy.post.mock.lastCall?.[1].body).toEqual({ sql: expect.stringContaining(`fromUnixTimestamp64Milli(${end})`) });
  await user.click(screen.getByRole("button", { name: "Refresh usage profiles" }));
  await waitFor(() => expect(proxy.post).toHaveBeenCalledTimes(2));
  expect(proxy.post.mock.lastCall?.[1].body).toEqual({ sql: expect.stringContaining(`fromUnixTimestamp64Milli(${end})`) });
});
