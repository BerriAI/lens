import { act, screen, within, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { chooseSelectOption, renderWithProviders, testQueryClient } from "../../../tests/test-utils";
import { readRequest, requestPath } from "../../../tests/lens-test-utils";
import { LensWorkspace } from "./LensWorkspace";
import { createLensDemoData } from "./data/demo/fixtures";
import type { LensList } from "./model/types";
import { rollUpAgents } from "./agents/agentRollup";

const network = vi.fn<typeof fetch>();
const list = vi.fn<() => Promise<LensList>>();
const data = createLensDemoData();
const worker = () => ({
  id: "setup-worker",
  analysis_key_id: "a".repeat(64),
  revoked: false,
  last_seen: new Date().toISOString(),
  scope: data.lenses[0].scope,
});

function serve({ enabled = false, traces = false, requests = false, connected = false, storageReady = true } = {}) {
  list.mockResolvedValue({ lenses: [], workers: connected ? [worker()] : [], tracing_enabled: enabled });
  network.mockImplementation(async (input, init) => {
    const { path, method, body, query } = await readRequest(input, init);
    if (path === "/lens/service") {
      const service = {
        url: "https://traces.test",
        configured: enabled || connected,
        connected: enabled,
        status: { storage_ready: storageReady, credentials_ready: true },
      };
      return Response.json(service);
    }
    if (path === "/v1/traces")
      return enabled
        ? Response.json({ data: traces ? [data.runs[0].trace.summary] : [] })
        : Response.json({ detail: "Tracing is not enabled" }, { status: 501 });
    if (path === "/v1/traces/agents")
      return Response.json({ agents: traces ? rollUpAgents([data.runs[0].trace.summary]) : [] });
    if (path === "/lens/activity/available") return Response.json({ traces, requests });
    if (path === "/lens/traces/findings") return Response.json([]);
    if (path === "/lens/feedback/summary") return Response.json([]);
    if (path === "/lens" && method === "POST") {
      const saved = { ...data.lenses[0], settings: { ...data.lenses[0].settings, ...(body as object) } };
      list.mockResolvedValue({ lenses: [saved], workers: [worker()], tracing_enabled: true });
      return Response.json(saved);
    }
    if (path === "/lens") return Response.json(await list());
    if (path === "/lens/models") return Response.json({ data: [{ id: "analysis" }] });
    if (path === "/lens/model_group/info")
      return Response.json({
        data: [{ model_group: "analysis", providers: ["OpenAI"], mode: "chat" }],
      });
    if (path === "/lens/signals") return Response.json({ model: "", threshold: 0.5, signals: [] });
    if (path === "/lens/agents") return Response.json(["support_agent"]);
    if (path === "/lens/preview/sample") return Response.json({ eligible: 1, selected: 1, executions: [] });
    if (path.endsWith("/reviews"))
      return Response.json({
        reviews: data.lenses[0].jobs[0].reviews.slice(Number(query.get("after") ?? 0)),
        reviewed: data.lenses[0].jobs[0].reviewed,
      });
    if (path.endsWith("/runs")) return Response.json(data.lenses[0].jobs);
    return Response.json({ data: [] });
  });
}

beforeEach(() => {
  testQueryClient.clear();
  window.localStorage.clear();
  window.sessionStorage.clear();
  network.mockReset();
  list.mockReset();
  vi.stubGlobal("fetch", network);
  Element.prototype.scrollIntoView = vi.fn();
  serve();
});

const renderWorkspace = (options?: Parameters<typeof renderWithProviders>[1], userRole = "Admin") =>
  renderWithProviders(<LensWorkspace accessToken="setup-token" userRole={userRole} readOnly={false} />, options);

const setupParam = (onUrlUpdate: ReturnType<typeof vi.fn>) =>
  new URLSearchParams(String(onUrlUpdate.mock.lastCall?.[0].queryString ?? "")).get("setup");

async function configureAnalysisFromSettings(user: ReturnType<typeof userEvent.setup>) {
  const settings = within(await screen.findByRole("region", { name: "Settings" }));
  expect(settings.getByRole("heading", { name: "Analysis", exact: true })).toBeVisible();
  list.mockResolvedValue({
    lenses: [],
    workers: [worker()],
    tracing_enabled: true,
  });
  await user.click(await settings.findByRole("button", { name: "Check configuration" }));
  expect(await settings.findByText("Analysis is configured")).toBeVisible();
  await user.click(settings.getByRole("button", { name: "View automatic analysis" }));
  expect(await screen.findByRole("region", { name: "Automatic analysis" })).toBeVisible();
  expect(screen.getByRole("tab", { name: "Findings" })).toHaveAttribute("aria-selected", "true");
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
}

describe("Lens introduction", () => {
  it("should open Home on a first visit and leave deployment setup separate from empty traces", async () => {
    window.localStorage.setItem("lens.intro.dismissed", "true");
    window.sessionStorage.setItem("lens.intro.seen", "true");
    const user = userEvent.setup();
    const first = renderWorkspace();
    const intro = await screen.findByRole("region", { name: "Get started with Lens" });
    expect(
      within(screen.getByRole("tabpanel", { name: "Home" })).getByRole("region", {
        name: "Get started with Lens",
      }),
    ).toBe(intro);
    expect(within(intro).getByRole("heading", { name: "Get your first trace" })).toBeVisible();
    expect(within(intro).getByRole("button", { name: "Copy setup instructions" })).toBeEnabled();
    expect(within(intro).queryByRole("textbox", { name: "Agent name" })).not.toBeInTheDocument();
    expect(within(intro).getAllByRole("button", { name: "Deployment setup" })[0]).toBeVisible();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Before you start" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: "Traces" }));
    expect(await screen.findByRole("region", { name: "Waiting for traces" })).toBeVisible();
    expect(screen.queryByRole("region", { name: "Get started with Lens" })).not.toBeInTheDocument();
    first.unmount();
    renderWorkspace();
    expect(await screen.findByRole("region", { name: "Get started with Lens" })).toBeVisible();
    expect(screen.getByRole("tab", { name: "Home" })).toHaveAttribute("aria-selected", "true");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("opens explicit setup links in the page and clears setup when navigating to Settings", async () => {
    serve({ enabled: true, traces: true });
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    renderWorkspace({ onUrlUpdate, searchParams: "?setup=lens" });
    expect(await screen.findByRole("region", { name: "Get started with Lens" })).toBeVisible();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: "Settings" }));
    expect(await screen.findByRole("region", { name: "Settings" })).toBeVisible();
    expect(screen.queryByRole("region", { name: "Get started with Lens" })).not.toBeInTheDocument();
    expect(screen.queryByRole("switch", { name: "Show the introduction on each new session" })).not.toBeInTheDocument();
    await waitFor(() => expect(setupParam(onUrlUpdate)).toBeNull());
  });

  it("never opens on its own inside the sample session", async () => {
    renderWorkspace({ searchParams: "?demo=true" });
    expect(await screen.findByText("Where is order #1042?")).toBeVisible();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});

describe("Lens setup journey", () => {
  it.each([false, true])("distinguishes an absent Lens service from a configured service failure (%s)", async (configured) => {
    serve();
    const normal = network.getMockImplementation()!;
    network.mockImplementation((input, init) =>
      requestPath(input) === "/lens/service"
        ? Promise.resolve(Response.json({ url: "", configured, connected: false, status: { storage_ready: false } }))
        : ["/lens", "/lens/activity/available"].includes(requestPath(input))
        ? Promise.resolve(Response.json({ detail: "Lens service unavailable" }, { status: 503 }))
        : normal(input, init),
    );
    const user = userEvent.setup();
    renderWorkspace({ searchParams: "?setup=lens" });
    await user.click(await screen.findByRole("button", { name: "Check setup" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Check setup" })).toBeEnabled());
    if (configured) {
      expect(await screen.findByText("Could not check setup. Lens service unavailable")).toBeVisible();
    } else {
      expect(screen.getByRole("link", { name: "Helm setup" })).toBeVisible();
      expect(screen.getByRole("button", { name: "Set it up for me" })).toBeEnabled();
      expect(screen.queryByText(/Could not check setup/)).not.toBeInTheDocument();
    }
  });

  it("should wait for storage readiness before completing explicit deployment setup", async () => {
    serve({ enabled: true, storageReady: false });
    const user = userEvent.setup();
    renderWorkspace({ searchParams: "?setup=lens" });
    const installation = await screen.findByRole("region", { name: /Install Lens/ });
    expect(await within(installation).findByText(/trace storage is unavailable/)).toBeVisible();
    expect(screen.queryByText("Trace storage is connected")).not.toBeInTheDocument();
    expect(screen.queryByRole("combobox", { name: "Your agent framework" })).not.toBeInTheDocument();
    serve({ enabled: true });
    await user.click(within(installation).getByRole("button", { name: "Check setup" }));
    expect(await screen.findByRole("combobox", { name: "Your agent framework" })).toBeVisible();
  });

  it.each(["/lens", "/lens/activity/available"])(
    "keeps recorded traces visible while %s is pending",
    async (pendingPath) => {
      serve({ enabled: true, traces: true });
      const normal = network.getMockImplementation()!;
      network.mockImplementation((input, init) =>
        requestPath(input) === pendingPath ? new Promise<Response>(() => {}) : normal(input, init),
      );
      renderWorkspace({ searchParams: "?tab=traces" });
      expect(await screen.findByRole("table", { name: "Agent runs" })).toBeVisible();
    },
  );

  it.each(["/v1/traces", "/lens/activity/available"])(
    "should open Findings from a saved link while %s is pending",
    async (pendingPath) => {
      serve();
      list.mockResolvedValue({ lenses: data.lenses, workers: [worker()], tracing_enabled: false });
      const normal = network.getMockImplementation()!;
      network.mockImplementation((input, init) =>
        requestPath(input) === pendingPath ? new Promise<Response>(() => {}) : normal(input, init),
      );
      renderWorkspace({ searchParams: `?lens=${data.lenses[0].id}` });
      expect(await screen.findByRole("region", { name: "Automatic analysis" })).toBeVisible();
      expect(screen.getByRole("tab", { name: "Findings" })).toHaveAttribute("aria-selected", "true");
      expect(await screen.findByRole("row", { name: data.lenses[0].findings[0].title })).toBeVisible();
    },
  );

  it("should open deployment setup from Home and continue through tracing into Findings", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    renderWorkspace({ onUrlUpdate });
    const home = within(await screen.findByRole("region", { name: "Get started with Lens" }));
    await user.click(home.getAllByRole("button", { name: "Deployment setup" })[0]);
    await waitFor(() => expect(setupParam(onUrlUpdate)).toBe("lens"));
    const intro = within(await screen.findByRole("region", { name: "Get started with Lens" }));
    serve({ enabled: true });
    await user.click(intro.getByRole("button", { name: "Check setup" }));
    expect(await intro.findByText("Trace storage is connected")).toBeVisible();
    await user.click(intro.getByRole("button", { name: "Continue to your agent" }));
    expect(intro.getByRole("button", { name: "Check for traces" })).toBeVisible();
    serve({ enabled: true, traces: true });
    await user.click(intro.getByRole("button", { name: "Check for traces" }));
    expect(await intro.findByText(/Your first trace is ready/)).toBeVisible();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.queryByRole("table", { name: "Agent runs" })).not.toBeInTheDocument();
    await user.click(intro.getByRole("button", { name: "Configure analysis" }));
    await waitFor(() => expect(setupParam(onUrlUpdate)).toBeNull());
    await configureAnalysisFromSettings(user);
  });

  it("continues to agent setup when a background service check detects the installation", async () => {
    const user = userEvent.setup();
    renderWorkspace({ searchParams: "?setup=lens" });
    const intro = within(await screen.findByRole("region", { name: "Get started with Lens" }));
    expect(await intro.findByRole("button", { name: "Check setup" })).toBeVisible();
    serve({ enabled: true });
    await act(() => testQueryClient.refetchQueries({ queryKey: ["lens-service"] }));
    await user.click(await intro.findByRole("button", { name: "Continue to your agent" }));
    expect(await intro.findByRole("combobox", { name: "Your agent framework" })).toBeVisible();
    expect(intro.getByRole("button", { name: "Generate tracing key" })).toBeEnabled();
    expect(intro.getByRole("button", { name: "Copy tracing configuration" })).toBeVisible();
    await chooseSelectOption(user, intro.getByRole("combobox", { name: "Your agent framework" }), "LangGraph");
    const normal = network.getMockImplementation()!;
    network.mockImplementation((input, init) =>
      requestPath(input) === "/lens/tracing/keys"
        ? Promise.resolve(Response.json({ key: "sk-tracing-setup", active: true }))
        : normal(input, init),
    );
    await user.click(intro.getByRole("button", { name: "Generate tracing key" }));
    expect(await intro.findByText("Your tracing key")).toBeVisible();
    serve({ enabled: true, storageReady: false });
    await act(() => testQueryClient.refetchQueries({ queryKey: ["lens-service"] }));
    const installation = within(await intro.findByRole("region", { name: /Install Lens/ }));
    expect(await installation.findByText(/trace storage is unavailable/)).toBeVisible();
    serve({ enabled: true });
    await act(() => testQueryClient.refetchQueries({ queryKey: ["lens-service"] }));
    const agent = within(await intro.findByRole("region", { name: /Send your first trace/ }));
    expect(await agent.findByRole("combobox", { name: "Your agent framework" })).toHaveTextContent("LangGraph");
    expect(agent.getByText("Your tracing key")).toBeVisible();
    expect(agent.queryByRole("button", { name: "Generate tracing key" })).not.toBeInTheDocument();
    serve({ enabled: true, traces: true });
    await user.click(intro.getByRole("button", { name: "Check for traces" }));
    expect(await intro.findByRole("button", { name: "Configure analysis" })).toBeEnabled();
  });

  it("resumes setup from the URL and leaves only when the user chooses traces", async () => {
    serve({ enabled: true, traces: true });
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    renderWorkspace({ searchParams: "?tab=investigations&setup=lens", onUrlUpdate });
    const intro = within(await screen.findByRole("region", { name: "Get started with Lens" }));
    expect(await intro.findByRole("button", { name: "Configure analysis" })).toBeVisible();
    expect(intro.getByRole("heading", { name: "Get Lens running" })).toBeVisible();
    await user.click(intro.getByRole("button", { name: "View traces" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(await screen.findByRole("table", { name: "Agent runs" })).toBeVisible();
    await waitFor(() => expect(setupParam(onUrlUpdate)).toBeNull());
    await user.click(screen.getByRole("tab", { name: "Findings" }));
    expect(await screen.findByRole("region", { name: "Automatic analysis" })).toBeVisible();
    expect(screen.queryByRole("region", { name: "Get Lens running" })).not.toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("should keep Findings available to request-only users without forcing agent instrumentation", async () => {
    serve({ requests: true });
    const user = userEvent.setup();
    const welcome = renderWorkspace({ searchParams: "?tab=investigations" });
    expect(await screen.findByRole("region", { name: "Automatic analysis" })).toBeVisible();
    expect(screen.getByRole("tab", { name: "Findings" })).toHaveAttribute("aria-selected", "true");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    welcome.unmount();
    renderWorkspace({ searchParams: "?tab=investigations&setup=lens" });
    const intro = within(await screen.findByRole("region", { name: "Get started with Lens" }));
    expect(await intro.findByRole("heading", { name: "Before you start" })).toBeVisible();
    expect(intro.getByRole("button", { name: "Configure analysis" })).toBeEnabled();
    await user.click(intro.getByRole("button", { name: /Install Lens/ }));
    expect(intro.getByRole("button", { name: "Configure analysis" })).toBeEnabled();
    await user.click(intro.getByRole("button", { name: /Send your first trace/ }));
    await user.click(intro.getByRole("button", { name: "Configure analysis" }));
    await configureAnalysisFromSettings(user);
  });

  it("keeps setup recoverable when checking for a first trace fails", async () => {
    serve({ enabled: true });
    const user = userEvent.setup();
    renderWorkspace({ searchParams: "?setup=lens" });
    const intro = within(await screen.findByRole("region", { name: "Get started with Lens" }));
    await intro.findByRole("button", { name: "Check for traces" });
    const normal = network.getMockImplementation()!;
    network.mockImplementation(async (input, init) => {
      const path = requestPath(input);
      if (path === "/v1/traces") return Response.json({ detail: "Trace storage unavailable" }, { status: 503 });
      return normal(input, init);
    });
    await user.click(intro.getByRole("button", { name: "Check for traces" }));
    expect(await intro.findByRole("alert")).toHaveTextContent("Could not check setup");
    expect(intro.queryByRole("button", { name: "Configure analysis" })).not.toBeInTheDocument();
    serve({ enabled: true, traces: true });
    await user.click(intro.getByRole("button", { name: "Retry" }));
    expect(await intro.findByRole("button", { name: "Configure analysis" })).toBeEnabled();
    expect(intro.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("should keep Findings available when an activity refresh fails", async () => {
    serve({ requests: true, connected: true });
    renderWorkspace({ searchParams: "?tab=investigations" });
    expect(await screen.findByRole("region", { name: "Automatic analysis" })).toBeVisible();
    const normal = network.getMockImplementation()!;
    network.mockImplementation((input, init) =>
      requestPath(input) === "/lens/activity/available"
        ? Promise.resolve(Response.json({ detail: "Activity unavailable" }, { status: 503 }))
        : normal(input, init),
    );
    await act(() => testQueryClient.refetchQueries());
    expect(screen.getByRole("region", { name: "Automatic analysis" })).toBeVisible();
    expect(screen.getByRole("tab", { name: "Findings" })).toHaveAttribute("aria-selected", "true");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    network.mockImplementation(normal);
    await act(() => testQueryClient.refetchQueries());
    expect(screen.getByRole("region", { name: "Automatic analysis" })).toBeVisible();
  });

  it("should keep administrator-only setup unavailable to trace viewers", async () => {
    serve({ enabled: true, traces: true });
    renderWorkspace({ searchParams: "?setup=lens" }, "Internal User");
    const intro = within(await screen.findByRole("region", { name: "Get started with Lens" }));
    expect(await intro.findByText("An administrator can configure automatic analysis.")).toBeVisible();
    expect(intro.getByRole("button", { name: "Configure analysis" })).toBeDisabled();
    expect(network.mock.calls.some(([input]) => requestPath(input) === "/lens")).toBe(false);
  });

  it.each(["traces", "requests with trace errors", "requests with pending traces", "traces with activity errors"])(
    "should finish guided setup with %s and open Findings without manual analysis setup",
    async (scenario) => {
      const source = scenario.startsWith("requests") ? "requests" : "traces";
      const activity = { enabled: true, traces: source === "traces", requests: source === "requests", connected: true };
      serve(activity);
      const normal = network.getMockImplementation()!;
      const failingPath = scenario === "requests with trace errors" ? "/v1/traces" : "/lens/activity/available";
      network.mockImplementation((input, init) => {
        const path = requestPath(input);
        if (path === "/v1/traces" && scenario === "requests with pending traces")
          return new Promise<Response>(() => {});
        if (scenario.endsWith("errors") && path === failingPath)
          return Promise.resolve(Response.json({ detail: "Activity unavailable" }, { status: 503 }));
        return normal(input, init);
      });
      const user = userEvent.setup();
      const onUrlUpdate = vi.fn();
      renderWorkspace({ searchParams: "?setup=lens", onUrlUpdate });
      const intro = within(await screen.findByRole("region", { name: "Get started with Lens" }));
      await user.click(await intro.findByRole("button", { name: "View automatic analysis" }));
      expect(await screen.findByRole("region", { name: "Automatic analysis" })).toBeVisible();
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
      expect(screen.queryByRole("region", { name: "New investigation" })).not.toBeInTheDocument();
      expect(
        within(screen.getByRole("tablist", { name: "Lens" })).getByRole("tab", { name: "Findings" }),
      ).toHaveAttribute("aria-selected", "true");
      await waitFor(() => expect(setupParam(onUrlUpdate)).toBeNull());
      const creates = network.mock.calls.filter(
        ([input, init]) => requestPath(input) === "/lens" && (init?.method ?? (input as Request).method) === "POST",
      );
      expect(creates).toHaveLength(0);
    },
  );
});
