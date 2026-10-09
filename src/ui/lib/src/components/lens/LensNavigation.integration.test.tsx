import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderWithProviders, testQueryClient } from "../../../tests/test-utils";
import { requestPath } from "../../../tests/lens-test-utils";
import { LensWorkspace } from "./LensWorkspace";

const admin = { accessToken: "test-token", userRole: "Admin", readOnly: false };
vi.mock("./traces/list/AgentTracesPage", () => ({
  default: ({ isActive }: { isActive: boolean }) => <div>Trace polling {isActive ? "active" : "paused"}</div>,
}));
vi.mock("./investigations/InvestigationsView", () => ({
  InvestigationsView: ({ readOnly }: { readOnly: boolean }) => (
    <div>{readOnly ? "Read-only investigations" : "Manage investigations"}</div>
  ),
}));

describe("Lens navigation", () => {
  beforeEach(() => {
    testQueryClient.clear();
    window.localStorage.clear();
    window.sessionStorage.clear();
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input) => {
        const path = requestPath(input);
        if (path === "/v1/traces") return Response.json({ data: [{}] });
        if (path === "/lens")
          return Response.json({
            lenses: [],
            workers: [],
            tracing_enabled: true,
          });
        return Response.json({ traces: true, requests: false, data: [] });
      }),
    );
  });

  it("keeps traces accessible and hides the separate investigations tab", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    renderWithProviders(<LensWorkspace {...admin} />, {
      onUrlUpdate,
      searchParams: "?tab=traces",
    });
    expect(screen.getByRole("tab", { name: "Traces" })).toHaveAttribute("aria-selected", "true");
    expect(await screen.findByText("Trace polling active")).toBeVisible();
    expect(screen.queryByRole("tab", { name: "Investigations" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: "Findings" }));
    expect(await screen.findByRole("region", { name: "Findings settings" })).toBeVisible();
    expect(screen.getByText("Trace polling paused")).not.toBeVisible();
    expect(onUrlUpdate.mock.lastCall?.[0].searchParams.get("tab")).toBe("findings");
    await user.click(screen.getByRole("tab", { name: "Traces" }));
    expect(await screen.findByText("Trace polling active")).toBeVisible();
  });

  it("should open existing lens links on Findings", async () => {
    renderWithProviders(<LensWorkspace {...admin} />, {
      searchParams: "?lens=saved-lens",
    });
    expect(screen.queryByRole("tab", { name: "Investigations" })).not.toBeInTheDocument();
    expect(await screen.findByRole("region", { name: "Findings settings" })).toBeVisible();
  });

  it("should connect a model from global Findings without selecting an agent", async () => {
    const user = userEvent.setup();
    renderWithProviders(<LensWorkspace {...admin} />, {
      searchParams: "?tab=findings",
    });
    expect(await screen.findByText("Connect an analysis model to start receiving findings.")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Connect model" }));
    expect(await screen.findByRole("region", { name: "Settings" })).toBeVisible();
    expect(screen.getByRole("tab", { name: "Settings" })).toHaveAttribute("aria-selected", "true");
  });

  it("honors an explicit traces tab even when a saved investigation is in the URL", async () => {
    renderWithProviders(<LensWorkspace {...admin} />, {
      searchParams: "?tab=traces&lens=saved-lens",
    });
    expect(screen.getByRole("tab", { name: "Traces" })).toHaveAttribute("aria-selected", "true");
    expect(await screen.findByText("Trace polling active")).toBeVisible();
  });

  it.each(["Internal User", "Internal Viewer", "Org Admin"])(
    "preserves trace access without granting investigations to %s",
    async (userRole) => {
      const user = userEvent.setup();
      renderWithProviders(<LensWorkspace {...admin} userRole={userRole} />, {
        searchParams: "?tab=traces",
      });
      expect(await screen.findByText("Trace polling active")).toBeVisible();
      await user.click(screen.getByRole("tab", { name: "Findings" }));
      expect(screen.getByText(/Findings require proxy administrator access/)).toBeVisible();
      expect(screen.queryByText("Manage investigations")).not.toBeInTheDocument();
    },
  );

  it.each([
    { userRole: "Admin Viewer", readOnly: false },
    { userRole: "Admin", readOnly: true },
  ])("should preserve read-only Findings access for $userRole with readOnly=$readOnly", async (session) => {
    renderWithProviders(<LensWorkspace {...admin} {...session} />, {
      searchParams: "?tab=investigations",
    });
    expect(await screen.findByRole("region", { name: "Findings settings" })).toBeVisible();
    expect(screen.queryByRole("button", { name: "Findings settings" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Connect model" })).not.toBeInTheDocument();
    expect(screen.queryByText("Manage investigations")).not.toBeInTheDocument();
  });
});
