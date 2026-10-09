import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { renderWithProviders } from "../../../../tests/test-utils";
import type { LensAgents } from "./AgentScoped";
import { AgentsView } from "./AgentsView";

const agents: LensAgents = {
  agent: "support_agent",
  select: vi.fn(),
  list: {
    isLoading: false,
    error: null,
    agents: [
      {
        name: "support_agent",
        runs: 240,
        failed_runs: 12,
        last_seen: "2026-10-08T12:00:00Z",
        frameworks: ["langgraph"],
      },
      {
        name: "research_agent",
        runs: 81,
        failed_runs: 0,
        last_seen: "2026-10-08T11:00:00Z",
        frameworks: [],
      },
    ],
  },
};

describe("Agents directory", () => {
  it("connects GitHub for the chosen agent without opening another agent’s traces", async () => {
    const user = userEvent.setup();
    const onOpenAgent = vi.fn();
    const onConnectGitHub = vi.fn();
    renderWithProviders(
      <AgentsView agents={agents} onOpenAgent={onOpenAgent} onConnectGitHub={onConnectGitHub} />,
    );

    await user.click(screen.getByRole("button", { name: "Connect GitHub for research_agent" }));
    expect(onConnectGitHub).toHaveBeenCalledExactlyOnceWith("research_agent");
    expect(onOpenAgent).not.toHaveBeenCalled();
  });

  it("should show agents with their run counts, errors, and framework", () => {
    renderWithProviders(<AgentsView agents={agents} onOpenAgent={vi.fn()} />);

    const table = screen.getByRole("table", { name: "Agents" });
    const support = within(table).getByRole("row", { name: /support_agent/ });
    expect(within(support).getByRole("cell", { name: "240" })).toBeVisible();
    expect(within(support).getByRole("cell", { name: "12" })).toBeVisible();
    expect(within(support).getByText("LangGraph")).toBeVisible();
    expect(within(table).getByRole("button", { name: "Open research_agent" })).toBeVisible();
  });

  it("should restore its shared search and open the chosen agent", async () => {
    const user = userEvent.setup();
    const onOpenAgent = vi.fn();
    const onUrlUpdate = vi.fn();
    renderWithProviders(<AgentsView agents={agents} onOpenAgent={onOpenAgent} />, {
      searchParams: "?tab=agents&agent_search=SUPPORT",
      onUrlUpdate,
    });

    expect(screen.getByRole("textbox", { name: "Search agents" })).toHaveValue("SUPPORT");
    expect(screen.queryByRole("button", { name: "Open research_agent" })).not.toBeInTheDocument();
    act(() =>
      fireEvent.change(screen.getByRole("textbox", { name: "Search agents" }), {
        target: { value: "research" },
      }),
    );
    await waitFor(() => expect(onUrlUpdate.mock.lastCall?.[0].searchParams.get("agent_search")).toBe("research"));
    expect(screen.queryByRole("button", { name: "Open support_agent" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Open research_agent" }));
    expect(onOpenAgent).toHaveBeenCalledExactlyOnceWith("research_agent");
  });

  it("should explain when no agents match the search", () => {
    renderWithProviders(<AgentsView agents={agents} onOpenAgent={vi.fn()} />, {
      searchParams: "?tab=agents&agent_search=missing",
    });

    expect(screen.getByText("No agents match “missing”")).toBeVisible();
    expect(screen.queryByRole("button", { name: /Open .*_agent/ })).not.toBeInTheDocument();
  });

  it("should distinguish an empty directory from loading and failure", () => {
    const onOpenAgent = vi.fn();
    const first = renderWithProviders(
      <AgentsView
        agents={{
          ...agents,
          list: { agents: [], isLoading: true, error: null },
        }}
        onOpenAgent={onOpenAgent}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent("Loading agents");
    expect(screen.queryByText("No agents yet")).not.toBeInTheDocument();

    first.rerender(
      <AgentsView
        agents={{
          ...agents,
          list: {
            agents: [],
            isLoading: false,
            error: new Error("Unavailable"),
          },
        }}
        onOpenAgent={onOpenAgent}
      />,
    );
    expect(screen.getByRole("alert")).toHaveTextContent("Could not load agents");
    expect(screen.queryByText("No agents yet")).not.toBeInTheDocument();

    first.rerender(
      <AgentsView
        agents={{
          ...agents,
          list: { agents: [], isLoading: false, error: null },
        }}
        onOpenAgent={onOpenAgent}
      />,
    );
    expect(screen.getByRole("heading", { name: "No agents yet" })).toBeVisible();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });
});
