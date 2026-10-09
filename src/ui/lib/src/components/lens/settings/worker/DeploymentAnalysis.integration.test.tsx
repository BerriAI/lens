import { screen, waitFor } from "@testing-library/react";
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
        if (path === "/lens/model_group/info") return { data: [model] };
        if (path === "/lens/signals") return { model: "", threshold: 0.5, signals: [] };
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
      expect(screen.getByRole("list", { name: "Analysis models" })).toHaveTextContent("analysis · openai");
      expect(gateway.get.mock.calls.map(([path]) => path).sort()).toEqual([
        "/lens",
        "/lens/model_group/info",
        "/lens/signals",
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
    expect(screen.getByRole("list", { name: "Analysis models" })).toHaveTextContent("analysis · openai");
    expect(screen.getByRole("button", { name: "New investigation" })).toBeVisible();
    expect(gateway.get.mock.calls.map(([path]) => path).sort()).toEqual(["/lens", "/lens/model_group/info"]);
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
      if (path === "/lens/model_group/info") throw new Error("Lens is temporarily unavailable");
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
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("waiting for the investigation worker"));
    expect(screen.queryByRole("button", { name: "New investigation" })).not.toBeInTheDocument();
  });
});
