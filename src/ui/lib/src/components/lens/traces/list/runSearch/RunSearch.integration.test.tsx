import { screen, within, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { runs } from "./__fixtures__/runs";
import { RunSearch } from "./RunSearch";
import { renderWithProviders as render } from "../../../../../../tests/test-utils";
import { stubGateway } from "../../../../../../tests/lens-test-utils";
import { useRunFilterRouting } from "../../routing";

const box = () => screen.getByRole("combobox", { name: "Search runs" });
const listbox = () =>
  screen.getByRole("listbox", { name: "Search suggestions" });

function RoutedSearch() {
  const { query, setQuery } = useRunFilterRouting();
  return (
    <RunSearch
      value={query}
      onChange={setQuery}
      runs={runs}
      range={{ startMs: 1000, endMs: 2000 }}
    />
  );
}

describe("RunSearch", () => {
  it("offers LiteLLM tags inside the search menu and carries the selection into investigation setup", async () => {
    const user = userEvent.setup();
    const gateway = stubGateway();
    gateway.post.mockResolvedValue({
      data: [{ tag: "skill:diagram" }, { tag: "User-Agent: codex" }],
    });
    const onUrlUpdate = vi.fn();
    render(<RoutedSearch />, {
      searchParams: "?agent=code-agent",
      onUrlUpdate,
    });
    await user.click(box());
    await user.click(
      within(listbox()).getByRole("option", { name: "tag", exact: true }),
    );
    await user.click(
      await screen.findByRole("option", { name: "skill:diagram", exact: true }),
    );
    await waitFor(() =>
      expect(onUrlUpdate.mock.lastCall?.[0].queryString).toContain("skill"),
    );
    expect(box()).toHaveTextContent("tag:skill:diagram");
    expect(
      screen.queryByRole("button", { name: "Tags" }),
    ).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Investigate tags" }));
    await waitFor(() =>
      expect(onUrlUpdate.mock.lastCall?.[0].queryString).toContain(
        "dialog=new",
      ),
    );
    const params = new URLSearchParams(
      onUrlUpdate.mock.lastCall?.[0].queryString,
    );
    expect(params.get("q")?.trim()).toBe("tag:skill:diagram");
    expect(params.get("agent")).toBe("code-agent");
    expect(
      screen.queryByRole("button", { name: "Copy as curl" }),
    ).not.toBeInTheDocument();
  });

  it("shows and removes tags from an existing shared link in the search area", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    render(<RoutedSearch />, {
      searchParams: '?tags=[{"key":"tag","value":"skill:diagram"}]',
      onUrlUpdate,
    });
    await user.click(
      screen.getByRole("button", {
        name: "Remove tag filter tag: skill:diagram",
      }),
    );
    await waitFor(() =>
      expect(
        screen.queryByRole("button", { name: "Investigate tags" }),
      ).not.toBeInTheDocument(),
    );
  });
  it("offers the run fields, then the agents seen in the loaded runs", async () => {
    const user = userEvent.setup();
    render(<RunSearch value="" onChange={vi.fn()} runs={runs} />);
    expect(screen.getByText("Search runs or filter by tag")).toBeVisible();
    await user.click(box());
    expect(
      within(listbox())
        .getAllByRole("group")
        .map((g) => g.getAttribute("aria-label")),
    ).toEqual(["Run attributes", "Content", "Identity"]);
    await user.keyboard("agent:");
    expect(
      within(listbox())
        .getAllByRole("option")
        .map((o) => o.textContent),
    ).toEqual(["billing-agent", "cron", "researcher", "triage"]);
    await user.click(
      within(listbox()).getByRole("option", { name: "researcher" }),
    );
    expect(box()).toHaveTextContent(/^agent:researcher $/, {
      normalizeWhitespace: false,
    });
  });

  it("copies the filtered list as a trace query bounded to the shown range", async () => {
    const user = userEvent.setup();
    const range = { startMs: 1_700_000_000_000, endMs: 1_700_003_600_000 };
    render(<RunSearch value="" onChange={vi.fn()} runs={runs} range={range} />);
    await user.click(box());
    await user.keyboard("agent:res");
    await user.click(screen.getByRole("button", { name: "Copy as curl" }));
    const command = await navigator.clipboard.readText();
    expect(command).toContain('/v1/traces/query"');
    expect(command).toContain("fromUnixTimestamp64Milli(1700000000000)");
    expect(command).toContain("fromUnixTimestamp64Milli(1700003600000)");
    expect(command).toContain("arrayExists(x -> x ILIKE 'res', agents)");
    expect(screen.getByRole("button", { name: "Copied" })).toBeVisible();
    expect(box()).toHaveAttribute("aria-expanded", "true");
  });
});
