import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderWithLens, stubGateway } from "../../../../../tests/lens-test-utils";
import { testQueryClient } from "../../../../../tests/test-utils";
import { DeploymentAnalysis } from "./DeploymentAnalysis";
import type { LensList } from "../../model/types";
import { LensHostProvider } from "../../../../host/LensHost";
import { LensSettings } from "../LensSettings";

const workers: LensList["workers"] = [
  {
    id: "local-worker",
    name: "Lens",
    last_seen: new Date().toISOString(),
    revoked: false,
    scope: { all_teams: true, team_id: "", api_key_hash: "" },
    analysis_key_id: "a".repeat(64),
  },
];
const model = { model_group: "analysis", providers: ["openai"], mode: "chat" };

describe("Deployment analysis setup", () => {
  beforeEach(() => testQueryClient.clear());

  it.each([{}, { surface: "embedded" }, { surface: "standalone" }] as const)(
    "uses deployment analysis with host %j and no enrollment options",
    async (host) => {
      const user = userEvent.setup();
      const gateway = stubGateway();
      const onConnectProject = vi.fn();
      const list: LensList = { lenses: [], workers, tracing_enabled: true };
      gateway.get.mockImplementation((path) => {
        if (path === "/lens") return list;
        if (path === "/lens/admin/model_group/info") return { data: [model] };
        if (path === "/lens/admin/signals") return { model: "", threshold: 0.5, signals: [] };
        if (path === "/lens/admin/signals/alerts")
          return { revision: 0, enabled: false, connected: false, signal_ids: [], last_delivery: null };
        if (path === "/lens/admin/gateway")
          return {
            configured: false,
            connected: false,
            api_base: null,
            analysis_models: 0,
            evaluation_models: 0,
            error: null,
            last_refreshed: null,
          };
        throw new Error(`Unsupported Lens request: ${path}`);
      });
      renderWithLens(
        <LensHostProvider host={host}>
          <LensSettings
            list={list}
            onConnectProject={onConnectProject}
            workerReadyAction={<button>New investigation</button>}
          />
        </LensHostProvider>,
      );
      expect(await screen.findByRole("heading", { name: "Analysis", exact: true })).toBeVisible();
      expect(await screen.findByRole("button", { name: "New investigation" })).toBeVisible();
      expect(screen.getByText("1 analysis model available")).toBeVisible();
      expect(screen.getByRole("button", { name: "Browse models" })).toBeVisible();
      expect(screen.queryByRole("list", { name: "Analysis models" })).not.toBeInTheDocument();
      expect(await screen.findByText("No model gateway configured")).toBeVisible();
      expect(gateway.get.mock.calls.map(([path]) => path).sort()).toEqual([
        "/lens",
        "/lens/admin/gateway",
        "/lens/admin/model_group/info",
      ]);
      expect(gateway.post).not.toHaveBeenCalled();
      await user.click(screen.getByRole("button", { name: "Connect project" }));
      expect(onConnectProject).toHaveBeenCalledOnce();
    },
  );

  it("reuses configured models and the installed worker without requesting gateway keys", async () => {
    const gateway = stubGateway();
    gateway.get.mockImplementation((path) => (path === "/lens" ? { workers } : { data: [model] }));
    renderWithLens(<DeploymentAnalysis workers={workers} readyAction={<button>New investigation</button>} />);
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("Analysis is configured"));
    expect(screen.getByText("1 analysis model available")).toBeVisible();
    expect(screen.getByRole("button", { name: "New investigation" })).toBeVisible();
    expect(gateway.get.mock.calls.map(([path]) => path).sort()).toEqual(["/lens", "/lens/admin/model_group/info"]);
    expect(gateway.post).not.toHaveBeenCalled();
  });

  it("resumes after deployment configuration changes and the user retries", async () => {
    const gateway = stubGateway();
    let configured = false;
    gateway.get.mockImplementation((path) =>
      path === "/lens" ? { workers: configured ? workers : [] } : { data: configured ? [model] : [] },
    );
    const user = userEvent.setup();
    renderWithLens(<DeploymentAnalysis workers={[]} readyAction={<button>New investigation</button>} />);
    expect(await screen.findByRole("heading", { name: "Add an analysis provider" })).toBeVisible();
    expect(screen.queryByRole("button", { name: "New investigation" })).not.toBeInTheDocument();
    configured = true;
    await user.click(screen.getByRole("button", { name: "Check configuration" }));
    expect(await screen.findByRole("button", { name: "New investigation" })).toBeVisible();
    expect(screen.getByRole("status")).toHaveTextContent("Analysis is configured");
  });

  it("distinguishes an unreachable check from missing configuration", async () => {
    const gateway = stubGateway();
    gateway.get.mockImplementation((path) => {
      if (path === "/lens/admin/model_group/info") throw new Error("Lens is temporarily unavailable");
      return { workers };
    });
    const user = userEvent.setup();
    renderWithLens(<DeploymentAnalysis workers={workers} />);
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not check analysis configuration");
    expect(screen.queryByRole("heading", { name: "Add an analysis provider" })).not.toBeInTheDocument();
    gateway.get.mockImplementation((path) => (path === "/lens" ? { workers } : { data: [model] }));
    await user.click(screen.getByRole("button", { name: "Check configuration" }));
    await waitFor(() => expect(screen.queryByRole("alert")).not.toBeInTheDocument());
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("Analysis is configured"));
  });

  it("keeps stale worker state distinct from a configured provider", async () => {
    const gateway = stubGateway();
    gateway.get.mockImplementation((path) =>
      path === "/lens" ? { workers: [{ ...workers[0], last_seen: "1970-01-01T00:00:00Z" }] } : { data: [model] },
    );
    renderWithLens(<DeploymentAnalysis workers={[]} readyAction={<button>New investigation</button>} />);
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("waiting for the analysis worker"));
    expect(screen.queryByRole("button", { name: "New investigation" })).not.toBeInTheDocument();
  });

  it("passes only eligible analysis models to the browser", async () => {
    const user = userEvent.setup();
    const gateway = stubGateway();
    gateway.get.mockImplementation((path) =>
      path === "/lens"
        ? { workers }
        : {
            data: [
              { model_group: "Zulu-chat", providers: ["gateway"], mode: "chat" },
              { model_group: "compatible-chat", providers: ["gateway"] },
              { model_group: "Beta-chat", providers: ["gateway"], mode: "chat" },
              model,
              { model_group: "embedding-model", providers: ["gateway"], mode: "embedding" },
              { model_group: "signal-model", providers: ["gateway"], mode: "evaluation" },
              { model_group: "image-model", providers: ["gateway"], mode: "image_generation" },
            ],
          },
    );
    renderWithLens(<DeploymentAnalysis workers={workers} />);
    expect(await screen.findByText("4 analysis models available")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Browse models" }));
    const models = await screen.findByRole("list", { name: "Analysis models" });
    expect(within(models).getAllByRole("listitem").map((item) => item.textContent)).toEqual([
      "analysis",
      "Beta-chat",
      "compatible-chat",
      "Zulu-chat",
    ]);
  });

  it("browses and searches a large catalog in a dismissible dialog without changing configuration", async () => {
    const user = userEvent.setup();
    const gateway = stubGateway();
    const catalog = Array.from({ length: 200 }, (_, index) => ({
      model_group: `analysis-${String(index).padStart(3, "0")}`,
      providers: ["openai_compatible"],
      mode: "chat",
    }));
    gateway.get.mockImplementation((path) => (path === "/lens" ? { workers } : { data: catalog }));
    renderWithLens(<DeploymentAnalysis workers={workers} />);
    expect(await screen.findByText("200 analysis models available")).toBeVisible();
    expect(screen.queryByText("analysis-199")).not.toBeInTheDocument();

    const browse = screen.getByRole("button", { name: "Browse models" });
    await user.click(browse);
    const dialog = await screen.findByRole("dialog", { name: "Analysis models" });
    const search = within(dialog).getByRole("searchbox", { name: "Search models" });
    expect(within(dialog).getAllByRole("listitem")).toHaveLength(200);
    expect(within(dialog).getByText("analysis-199")).toBeInTheDocument();
    expect(within(dialog).queryByText("openai_compatible")).not.toBeInTheDocument();
    await waitFor(() => expect(search).toHaveFocus());
    await user.tab();
    expect(within(dialog).getByRole("region", { name: "Model results" })).toHaveFocus();

    fireEvent.change(search, { target: { value: "  ANALYSIS-19  " } });
    expect(within(dialog).getAllByRole("listitem")).toHaveLength(10);
    expect(within(dialog).getByRole("status")).toHaveTextContent("10 of 200 models");
    expect(within(dialog).getByText("analysis-199")).toBeVisible();
    expect(within(dialog).queryByText("analysis-001")).not.toBeInTheDocument();

    fireEvent.change(search, { target: { value: "unavailable" } });
    expect(within(dialog).getByText("No models match your search")).toBeVisible();
    expect(within(dialog).getByRole("status")).toHaveTextContent("0 of 200 models");
    await user.click(search);
    await user.tab();
    expect(within(dialog).getByRole("button", { name: "Close", exact: true })).toHaveFocus();
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(browse).toHaveFocus();

    await user.click(browse);
    const reopened = await screen.findByRole("dialog", { name: "Analysis models" });
    expect(within(reopened).getByRole("searchbox", { name: "Search models" })).toHaveValue("");
    expect(within(reopened).getAllByRole("listitem")).toHaveLength(200);
    await user.click(within(reopened).getByRole("button", { name: "Close", exact: true }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(gateway.post).not.toHaveBeenCalled();
  });
});
