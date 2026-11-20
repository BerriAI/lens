import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { renderWithProviders } from "../../../../tests/test-utils";
import { useWorkspaceScope } from "../route";
import { WorkspaceSwitchers, type WorkspacePicker } from "./WorkspaceSwitchers";

const teams = [
  { id: "team-a", name: "Platform" },
  { id: "team-b", name: "Engineering" },
];

describe("WorkspaceSwitchers", () => {
  it("identifies the workspace with one Lens wordmark", () => {
    renderWithProviders(<WorkspaceSwitchers pickers={[]} onSelect={vi.fn()} />);
    expect(screen.getByRole("heading", { name: "Lens", level: 1 })).toBeVisible();
    expect(screen.queryByRole("img", { name: "LiteLLM" })).not.toBeInTheDocument();
  });

  it("searches by the visible name and selects with the keyboard", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    renderWithProviders(
      <WorkspaceSwitchers
        pickers={[{ kind: "team", value: teams[0], items: teams }]}
        onSelect={onSelect}
      />,
    );
    await user.click(screen.getByRole("combobox", { name: "Team workspace" }));
    await user.type(
      await screen.findByRole("combobox", { name: "Search teams" }),
      "Engineering",
    );
    expect(
      await screen.findByRole("option", { name: "Engineering" }),
    ).toBeVisible();
    expect(
      screen.queryByRole("option", { name: "Platform" }),
    ).not.toBeInTheDocument();
    await user.keyboard("{ArrowDown}{Enter}");
    expect(onSelect).toHaveBeenCalledWith("team", teams[1]);
  });

  it("returns to all teams and exposes a retry after a failed lookup", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    const onRetry = vi.fn();
    const { rerender } = renderWithProviders(
      <WorkspaceSwitchers
        pickers={[{ kind: "team", value: teams[0], items: teams }]}
        onSelect={onSelect}
      />,
    );
    await user.click(screen.getByRole("combobox", { name: "Team workspace" }));
    await user.click(await screen.findByRole("option", { name: "All teams" }));
    expect(onSelect).toHaveBeenCalledWith("team", null);
    rerender(
      <WorkspaceSwitchers
        pickers={[
          {
            kind: "team",
            value: teams[0],
            items: [],
            error: "Could not load teams",
            onRetry,
          },
        ]}
        onSelect={onSelect}
      />,
    );
    await user.click(screen.getByRole("combobox", { name: "Team workspace" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Could not load teams",
    );
    await user.click(screen.getByRole("button", { name: "Retry" }));
    expect(onRetry).toHaveBeenCalledOnce();
  });

  it("identifies a key without a project in its hierarchy", () => {
    const pickers: WorkspacePicker[] = [
      { kind: "project", value: null, items: [] },
      {
        kind: "key",
        value: { id: "hash-a", name: "Development", project_id: null },
        items: [],
      },
    ];
    renderWithProviders(
      <WorkspaceSwitchers pickers={pickers} onSelect={vi.fn()} />,
    );
    expect(
      screen.getByRole("combobox", { name: "Project workspace" }),
    ).toHaveTextContent("No project");
    expect(
      screen.getByRole("combobox", { name: "Key workspace" }),
    ).toHaveTextContent("Development");
  });
});

it("switches scope as one history entry while retaining the current tab and date", async () => {
  function Switcher() {
    const { select } = useWorkspaceScope();
    return (
      <button
        onClick={() =>
          select("team", {
            id: "team-b",
            name: "Engineering",
            organization_id: "org-b",
          })
        }
      >
        Switch team
      </button>
    );
  }
  const user = userEvent.setup();
  const onUrlUpdate = vi.fn();
  renderWithProviders(<Switcher />, {
    searchParams:
      "?tab=traces&hours=168&organization_id=org-a&team_id=team-a&project_id=project-a&api_key_hash=hash-a&agent=old-agent&trace=old-trace&span=old-span",
    onUrlUpdate,
  });
  await user.click(screen.getByRole("button", { name: "Switch team" }));
  await waitFor(() => expect(onUrlUpdate).toHaveBeenCalledOnce());
  const update = onUrlUpdate.mock.lastCall?.[0];
  const params = new URLSearchParams(update.queryString);
  expect(params.get("organization_id")).toBe("org-b");
  expect(params.get("team_id")).toBe("team-b");
  expect(params.get("tab")).toBe("traces");
  expect(params.get("hours")).toBe("168");
  expect(
    ["project_id", "api_key_hash", "agent", "trace", "span"].filter((key) =>
      params.has(key),
    ),
  ).toEqual([]);
  expect(update.options.history).toBe("push");
});

it("retains resolved parents when broadening a key-only deep link", async () => {
  function Switcher() {
    const { select } = useWorkspaceScope();
    return (
      <button
        onClick={() =>
          select("key", null, {
            organization_id: "org-a",
            team_id: "team-a",
            project_id: "project-a",
          })
        }
      >
        All keys
      </button>
    );
  }
  const user = userEvent.setup();
  const onUrlUpdate = vi.fn();
  renderWithProviders(<Switcher />, {
    searchParams: "?tab=traces&api_key_hash=hash-a",
    onUrlUpdate,
  });
  await user.click(screen.getByRole("button", { name: "All keys" }));
  await waitFor(() => expect(onUrlUpdate).toHaveBeenCalledOnce());
  const params = new URLSearchParams(
    onUrlUpdate.mock.lastCall?.[0].queryString,
  );
  expect(Object.fromEntries(params)).toEqual({
    tab: "traces",
    organization_id: "org-a",
    team_id: "team-a",
    project_id: "project-a",
  });
});
