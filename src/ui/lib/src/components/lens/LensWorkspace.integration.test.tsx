import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ThemeProvider } from "next-themes";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderWithProviders, testQueryClient } from "../../../tests/test-utils";
import { readRequest, requestPath } from "../../../tests/lens-test-utils";
import { LensWorkspace } from "./LensWorkspace";
import { lensKeys } from "./data/queries";
import { createLensDemoData } from "./data/demo/fixtures";
import { LensHostProvider } from "../../host/LensHost";

const lastUrl = (onUrlUpdate: ReturnType<typeof vi.fn>) =>
  new URLSearchParams(String(onUrlUpdate.mock.lastCall?.[0].queryString ?? ""));
const expectUrl = (onUrlUpdate: ReturnType<typeof vi.fn>, check: (params: URLSearchParams) => void) =>
  waitFor(() => check(lastUrl(onUrlUpdate)));

const defaultResponse = (path: string, connected: boolean) =>
  Response.json(
    path === "/lens/service"
      ? {
          configured: connected,
          connected,
          url: "https://lens.test",
          status: { storage_ready: connected, credentials_ready: true },
        }
      : { data: [], traces: connected, requests: false },
  );

const network = vi.fn<typeof fetch>();
beforeEach(() => {
  testQueryClient.clear();
  window.localStorage.clear();
  window.sessionStorage.clear();
  vi.stubGlobal("fetch", network);
  network.mockReset();
  network.mockImplementation(async (input) => {
    const path = requestPath(input);
    if (path === "/v1/traces") return Response.json({ detail: "Tracing is not enabled" }, { status: 501 });
    if (path === "/lens") return Response.json({ lenses: [], workers: [], tracing_enabled: false });
    if (path === "/v1/traces/agents") return Response.json({ agents: [] });
    if (path === "/lens/service")
      return Response.json({ configured: false, connected: false, url: "", status: { storage_ready: false } });
    return defaultResponse(path, false);
  });
});

it.each(["home", "agents", "traces"])("offers setup instructions immediately from %s", async (tab) => {
  const user = userEvent.setup();
  const onUrlUpdate = vi.fn();
  renderWithProviders(<LensWorkspace accessToken="live-token" userRole="Admin" readOnly={false} />, {
    searchParams: `?tab=${tab}`,
    onUrlUpdate,
  });
  if (tab !== "home") {
    await user.click(await screen.findByRole("button", { name: "Connect project" }));
    await expectUrl(onUrlUpdate, (url) => expect(url.get("tab")).toBe("home"));
  }
  expect(await screen.findByRole("heading", { name: "Get your first trace" })).toBeVisible();
  expect(screen.getByRole("tab", { name: "Home", selected: true })).toBeVisible();
  expect(screen.queryByRole("textbox", { name: "Agent name" })).not.toBeInTheDocument();
  expect(screen.queryByRole("combobox", { name: "Your project" })).not.toBeInTheDocument();
  expect(screen.queryByRole("navigation", { name: "Setup progress" })).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Copy setup instructions" }));
  const instructions = await navigator.clipboard.readText();
  expect(instructions).toContain("Determine the agent name from the project");
  expect(instructions).toContain("/v1/traces/receipt");
  expect(instructions).toContain("What would you like to instrument next?");
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(screen.queryByRole("region", { name: "Get Lens running" })).not.toBeInTheDocument();
  expect(screen.queryByRole("heading", { name: "The gateway that helps your agents improve" })).not.toBeInTheDocument();
  const requests = await Promise.all(network.mock.calls.map(([input, init]) => readRequest(input, init)));
  expect(requests.filter(({ method }) => method !== "GET")).toEqual([]);
});

it("should keep Home selected when connected agents load and offer their directory", async () => {
  const user = userEvent.setup();
  const onUrlUpdate = vi.fn();
  window.localStorage.setItem("litellm.lens.agent", "moyai");
  network.mockImplementation(async (input) => {
    const path = requestPath(input);
    if (path === "/v1/traces/agents")
      return Response.json({
        agents: [{ name: "moyai", runs: 1, failed_runs: 0, frameworks: [], last_seen: new Date().toISOString() }],
      });
    if (path === "/lens") return Response.json({ lenses: [], workers: [], tracing_enabled: true });
    return defaultResponse(path, true);
  });
  renderWithProviders(<LensWorkspace accessToken="live-token" userRole="Admin" readOnly={false} />, { onUrlUpdate });
  expect(await screen.findByRole("heading", { name: "Get your first trace" })).toBeVisible();
  expect(screen.getByRole("tab", { name: "Home", selected: true })).toBeVisible();
  expect(onUrlUpdate).not.toHaveBeenCalled();
  await user.click(await screen.findByRole("button", { name: "View agents" }));
  expect(await screen.findByRole("button", { name: "Open moyai" })).toBeVisible();
  await expectUrl(onUrlUpdate, (url) => expect(url.get("tab")).toBe("agents"));
});

it("returns from GitHub to the callback agent’s repository picker and clears the authorization after saving", async () => {
  const user = userEvent.setup();
  const onUrlUpdate = vi.fn();
  const connect = vi.fn();
  const statusAgent = vi.fn();
  const connection = {
    agent: "qa-agent",
    repository_id: 101,
    repository: "lens-test/agent",
    installation_id: 17,
    default_branch: "main",
    connected_at: "2026-10-09T19:00:00Z",
    available: true,
  };
  let connected = false;
  network.mockImplementation(async (input, init) => {
    const request = await readRequest(input, init);
    if (request.path === "/lens/github/status") {
      statusAgent(request.query.get("agent"));
      return Response.json({ configured: true, app_slug: "lens-qa", connection: connected ? connection : null });
    }
    if (request.path === "/lens/github/authorizations/qa-callback")
      return Response.json({ status: "ready", repositories: [{ id: 101, full_name: "lens-test/agent", installation_id: 17, default_branch: "main" }] });
    if (request.path === "/lens/github/connections/qa-agent" && request.method === "PUT") {
      connect(request.body);
      connected = true;
      return Response.json(connection);
    }
    if (request.path === "/v1/traces/agents")
      return Response.json({ agents: ["other-agent", "qa-agent"].map((name) => ({ name, runs: 1, failed_runs: 0, frameworks: [], last_seen: new Date().toISOString() })) });
    if (request.path === "/lens") return Response.json({ lenses: [], workers: [], tracing_enabled: true });
    return Response.json({ data: [], traces: true, requests: false });
  });
  renderWithProviders(<LensWorkspace accessToken="live-token" userRole="Admin" readOnly={false} />, {
    searchParams: "?tab=agents&agent=other-agent&github_agent=qa-agent&github_authorization=qa-callback",
    onUrlUpdate,
  });

  const dialog = await screen.findByRole("dialog", { name: "Connect GitHub" });
  expect(dialog).toHaveTextContent("Connect qa-agent to a repository through the Lens GitHub App");
  expect(await within(dialog).findByRole("combobox", { name: "GitHub repository" })).toHaveTextContent("lens-test/agent");
  await user.click(within(dialog).getByRole("button", { name: "Connect repository" }));
  expect(await within(dialog).findByRole("heading", { name: "GitHub connected" })).toBeVisible();
  expect(connect).toHaveBeenCalledExactlyOnceWith({ authorization_id: "qa-callback", repository_id: 101 });
  expect(statusAgent).toHaveBeenCalledWith("qa-agent");
  expect(statusAgent).not.toHaveBeenCalledWith("other-agent");
  await expectUrl(onUrlUpdate, (url) => expect(url.has("github_authorization")).toBe(false));
  expect(lastUrl(onUrlUpdate).get("github_agent")).toBe("qa-agent");
  expect(lastUrl(onUrlUpdate).get("agent")).toBe("other-agent");
  await user.keyboard("{Escape}");
  await expectUrl(onUrlUpdate, (url) => expect(url.has("github_agent")).toBe(false));
});

it("discovers a new agent and offers GitHub connection after Home receives a new trace", async () => {
  const user = userEvent.setup();
  const onUrlUpdate = vi.fn();
  const githubAgent = vi.fn();
  const agent = { name: "qa-agent", runs: 1, failed_runs: 0, frameworks: [], last_seen: new Date().toISOString() };
  const listAgents = vi.fn(() => ({ agents: [] as (typeof agent)[] }));
  network.mockImplementation(async (input, init) => {
    const request = await readRequest(input, init);
    if (request.path === "/v1/traces/agents") return Response.json(listAgents());
    if (request.path === "/lens") return Response.json({ lenses: [], workers: [], tracing_enabled: true });
    if (request.path === "/lens/github/status") {
      githubAgent(request.query.get("agent"));
      return Response.json({ configured: false, app_slug: null, connection: null });
    }
    return defaultResponse(request.path, true);
  });
  renderWithProviders(<LensWorkspace accessToken="live-token" userRole="Admin" readOnly={false} />, {
    searchParams: "?tab=home",
    onUrlUpdate,
  });

  const connection = within(await screen.findByRole("region", { name: "Live connection" }));
  await connection.findByText("Waiting for traces");
  expect(connection.queryByRole("button", { name: "Connect GitHub" })).not.toBeInTheDocument();
  await user.click(connection.getByRole("button", { name: "Connection details" }));
  listAgents.mockReturnValue({ agents: [{ ...agent, last_seen: new Date().toISOString() }] });
  await user.click(await connection.findByRole("button", { name: "Check connection" }));
  expect(await connection.findByText("New trace received")).toBeVisible();
  expect(connection.getByText(/Your coding agent will verify the run it sent/)).toHaveTextContent(
    `A trace arrived from “${agent.name}”.`,
  );
  expect(connection.getByText(/Your coding agent will verify the run it sent/)).toHaveTextContent("What would you like to instrument next?");
  await user.click(connection.getByRole("button", { name: "Connect GitHub" }));

  const dialog = await screen.findByRole("dialog", { name: "Connect GitHub" });
  expect(dialog).toHaveTextContent("Connect qa-agent to a repository through the Lens GitHub App");
  expect(await within(dialog).findByRole("heading", { name: "GitHub connection unavailable" })).toBeVisible();
  expect(githubAgent).toHaveBeenCalledExactlyOnceWith(agent.name);
  await expectUrl(onUrlUpdate, (url) => expect(url.get("github_agent")).toBe(agent.name));
  expect(lastUrl(onUrlUpdate).get("tab")).toBe("home");
  expect(lastUrl(onUrlUpdate).has("connect_agent")).toBe(false);
  expect(lastUrl(onUrlUpdate).has("connect_step")).toBe(false);
});

it("opens instructions from a legacy setup link in Traces without creating another tracing key", async () => {
  const user = userEvent.setup();
  const onUrlUpdate = vi.fn();
  network.mockImplementation(async (input) => {
    const path = requestPath(input);
    if (path === "/lens") return Response.json({ lenses: [], workers: [], tracing_enabled: true });
    if (path === "/v1/traces") return Response.json({ data: [], next_cursor: null });
    if (path === "/v1/traces/agents") return Response.json({ agents: [] });
    return defaultResponse(path, true);
  });
  renderWithProviders(<LensWorkspace accessToken="live-token" userRole="Admin" readOnly={false} />, {
    searchParams: "?tab=traces&connect_agent=support-agent&connect_step=verify",
    onUrlUpdate,
  });
  const waiting = within(await screen.findByRole("region", { name: "Waiting for traces" }));
  expect(waiting.getByRole("heading", { name: "Your first trace will appear here" })).toBeVisible();
  expect(waiting.getByText("support-agent")).toBeVisible();
  await user.click(waiting.getByRole("button", { name: "Continue setup" }));
  expect(await screen.findByRole("button", { name: "Copy setup instructions" })).toBeVisible();
  expect(screen.getByRole("tab", { name: "Home", selected: true })).toBeVisible();
  expect(screen.queryByRole("navigation", { name: "Setup progress" })).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Copy setup instructions" }));
  expect(await navigator.clipboard.readText()).toContain('Setup link agent name: "support-agent"');
  await user.click(screen.getByRole("tab", { name: "Traces" }));
  expect(await screen.findByRole("heading", { name: "Your first trace will appear here" })).toBeVisible();
  await user.click(screen.getByRole("tab", { name: "Home" }));
  expect(await screen.findByRole("button", { name: "Copy setup instructions" })).toBeVisible();
  await expectUrl(onUrlUpdate, (url) => expect(url.get("connect_step")).toBe("verify"));
  const requests = await Promise.all(network.mock.calls.map(([input, init]) => readRequest(input, init)));
  expect(requests.filter(({ path, method }) => path === "/lens/tracing/keys" && method === "POST")).toEqual([]);
});

it("keeps installation controls mounted while readiness retries and continues after Lens connects", async () => {
  const user = userEvent.setup();
  const unavailable = () => Response.json({ detail: "Configure Lens" }, { status: 503 });
  const listResponse = vi.fn<() => Promise<Response>>(async () => unavailable());
  const traceResponse = vi.fn(() => Response.json({ detail: "Tracing is not enabled" }, { status: 501 }));
  const serviceResponse = vi.fn(() =>
    Response.json({ configured: false, connected: false, url: "", status: { storage_ready: false } }),
  );
  network.mockImplementation(async (input) => {
    const path = requestPath(input);
    if (path === "/lens") return listResponse();
    if (path === "/v1/traces") return traceResponse();
    if (path === "/lens/service") return serviceResponse();
    if (path === "/v1/traces/agents") return Response.json({ agents: [] });
    return defaultResponse(path, false);
  });
  renderWithProviders(
    <LensHostProvider host={{ surface: "embedded", analysis: "deployment" }}>
      <LensWorkspace accessToken="live-token" userRole="Admin" readOnly={false} />
    </LensHostProvider>,
  );
  const setup = within(await screen.findByRole("region", { name: "Get Lens running" }));
  expect(await setup.findByRole("link", { name: "Helm setup" })).toBeVisible();
  const copy = setup.getByRole("button", { name: "Set it up for me" });
  const retry = Promise.withResolvers<Response>();
  listResponse.mockImplementationOnce(() => retry.promise);

  act(() => {
    void testQueryClient.refetchQueries({ queryKey: lensKeys.lists() });
  });
  await waitFor(() => expect(listResponse).toHaveBeenCalledTimes(2));
  expect(copy).toBeVisible();
  expect(screen.queryByText("Checking Lens setup…")).not.toBeInTheDocument();
  await user.click(copy);
  expect(await navigator.clipboard.readText()).toContain("Lens connection is not configured");
  await act(async () => retry.resolve(unavailable()));

  listResponse.mockImplementation(async () => Response.json({ lenses: [], workers: [], tracing_enabled: true }));
  traceResponse.mockImplementation(() => Response.json({ data: [], next_cursor: null }));
  serviceResponse.mockImplementation(() =>
    Response.json({ configured: true, connected: true, url: "https://lens.test", status: { storage_ready: true } }),
  );
  await user.click(await setup.findByRole("button", { name: "Check setup" }));
  await user.click(await setup.findByRole("button", { name: "Continue to your agent" }));
  expect(await setup.findByRole("button", { name: "Generate tracing key" })).toBeVisible();
  expect(setup.getByRole("button", { name: /Send your first trace/ })).toHaveAttribute("aria-expanded", "true");
});

describe("Lens interactive demo", () => {
  it("opens without tracing, keeps the sample session in the URL, and restores the live view without mixing data", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    window.localStorage.clear();
    renderWithProviders(<LensWorkspace accessToken="live-token" userRole="Internal User" readOnly={false} />, {
      onUrlUpdate,
    });
    expect(await screen.findByRole("heading", { name: "Get your first trace" })).toBeVisible();
    expect(screen.queryByRole("button", { name: "Preview sample" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("switch", { name: "Demo data" }));
    expect(await screen.findByText("Where is order #1042?")).toBeVisible();
    expect(screen.getByRole("switch", { name: "Demo data" })).toBeChecked();
    await expectUrl(onUrlUpdate, (url) => expect(url.get("demo")).toBe("true"));
    expect(screen.getByRole("button", { name: "Connect project" })).toBeDisabled();
    network.mockClear();
    expect(screen.getByText("Where is order #1042?")).toBeVisible();
    const search = screen.getByRole("combobox", { name: "Search runs" });
    await user.type(search, "headphones");
    await waitFor(() => expect(screen.queryByText("Where is order #1042?")).not.toBeInTheDocument());
    expect(screen.getByText("Can I return my headphones?")).toBeVisible();
    await user.clear(search);
    await user.type(search, "agent:support_agent status:error");
    const table = screen.getByRole("table", { name: "Agent runs" });
    await waitFor(() => expect(within(table).getAllByRole("row")).toHaveLength(4));
    await expectUrl(onUrlUpdate, (url) => expect(url.get("q")).toBe("agent:support_agent status:error"));
    await user.click(screen.getByRole("tab", { name: "Investigations" }));
    expect(await screen.findByRole("row", { name: /Support quality/ })).toBeVisible();
    expect(screen.queryByRole("button", { name: "New investigation" })).not.toBeInTheDocument();
    expect(network).not.toHaveBeenCalled();
    await user.click(screen.getByRole("switch", { name: "Demo data" }));
    expect(await screen.findByText(/Investigations require proxy administrator access/)).toBeVisible();
    expect(screen.queryByText("Can I return my headphones?")).not.toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "Demo data" })).not.toBeChecked();
    await expectUrl(onUrlUpdate, (url) => expect([...url.entries()]).toEqual([["tab", "investigations"]]));
  });

  it("opens the sample session from ?demo=true and keeps the open run and step in the URL", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    renderWithProviders(<LensWorkspace accessToken="live-token" userRole="Internal User" readOnly={false} />, {
      searchParams: "?demo=true",
      onUrlUpdate,
    });
    expect(await screen.findByRole("switch", { name: "Demo data" })).toBeChecked();
    await user.click(await screen.findByText("Where is order #1042?"));
    const drawer = await screen.findByRole("complementary", { name: "Trace details" });
    await expectUrl(onUrlUpdate, (url) => expect(url.get("trace")).toBeTruthy());
    const openRun = lastUrl(onUrlUpdate);
    expect(openRun.get("demo")).toBe("true");
    const run = createLensDemoData().runs.find(({ trace }) => trace.summary.trace_id === openRun.get("trace"));
    const steps = await within(drawer).findAllByRole("treeitem");
    const step = steps.find((row) => run?.trace.spans.some((span) => span.span_id === row.getAttribute("data-row-id")));
    await user.click(step!);
    await expectUrl(onUrlUpdate, (url) => expect(url.get("span")).toBe(step!.getAttribute("data-row-id")));
    expect(lastUrl(onUrlUpdate).get("trace")).toBe(openRun.get("trace"));
    await user.click(within(drawer).getByRole("tab", { name: "Attributes" }));
    await expectUrl(onUrlUpdate, (url) => expect(url.get("span_tab")).toBe("attributes"));
    await user.click(within(drawer).getByRole("tab", { name: "Thread" }));
    await expectUrl(onUrlUpdate, (url) => expect(url.get("view")).toBe("thread"));
    expect(network).not.toHaveBeenCalled();
    await user.click(screen.getByRole("switch", { name: "Demo data" }));
    expect(await screen.findByRole("region", { name: "Get started with Lens" })).toBeVisible();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    await expectUrl(onUrlUpdate, (url) => expect([...url.keys()]).toEqual([]));
  });

  it("drops the live run selection when entering the sample session", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    network.mockResolvedValue(Response.json({ detail: "Tracing is not enabled" }, { status: 501 }));
    renderWithProviders(<LensWorkspace accessToken="live-token" userRole="Internal User" readOnly={false} />, {
      searchParams: "?tab=traces&setup=lens&trace=live-trace&span=live-span&lens=live-lens",
      onUrlUpdate,
    });
    await user.click(await screen.findByRole("switch", { name: "Demo data" }));
    expect(await screen.findByText("Where is order #1042?")).toBeVisible();
    await expectUrl(onUrlUpdate, (url) =>
      expect([...url.entries()]).toEqual([
        ["tab", "traces"],
        ["demo", "true"],
        ["agent", "support_agent"],
      ]),
    );
    expect(screen.queryByText(/Could not load trace/)).not.toBeInTheDocument();
  });

  it("reopens a shared sample link on the same run, step and section", async () => {
    const data = createLensDemoData();
    const run = data.runs[0].trace;
    const step = run.spans[run.spans.length - 1];
    renderWithProviders(<LensWorkspace accessToken="live-token" userRole="Internal User" readOnly={false} />, {
      searchParams: `?demo=true&trace=${run.summary.trace_id}&span=${step.span_id}&span_tab=request`,
    });
    const drawer = await screen.findByRole("complementary", { name: "Trace details" });
    expect(await within(drawer).findByRole("treeitem", { selected: true })).toHaveAttribute(
      "data-row-id",
      step.span_id,
    );
    expect(within(drawer).getByRole("tab", { name: "Request" })).toHaveAttribute("aria-selected", "true");
  });

  it("connects findings and history to their original trace without live requests", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    window.localStorage.clear();
    renderWithProviders(<LensWorkspace accessToken="live-token" userRole="Admin" readOnly={false} />, {
      searchParams: "?tab=findings",
      onUrlUpdate,
    });
    await user.click(await screen.findByRole("switch", { name: "Demo data" }));
    network.mockClear();
    await user.click(await screen.findByRole("row", { name: /Repeated lookups leave customers without an answer/ }));
    const finding = screen.getByRole("complementary", { name: "Finding details" });
    expect(within(finding).getByText(/The support agent retries/)).toBeVisible();
    await user.click(within(finding).getAllByRole("button", { name: "View span" })[0]);
    expect(await screen.findByRole("complementary", { name: "Span details" })).toHaveTextContent(
      "I will check that for you.",
    );
    await user.click(screen.getByRole("button", { name: "Copy for agent" }));
    expect(await navigator.clipboard.readText()).toContain("I will check that for you.");
    expect(await navigator.clipboard.readText()).not.toContain("Authorization");
    await user.click(screen.getByRole("tab", { name: "Attributes" }));
    expect(await screen.findByText("gen_ai.agent.name")).toBeVisible();
    expect(within(finding).getByRole("button", { name: "Back to finding" })).toBeVisible();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    await user.click(within(finding).getByRole("button", { name: "Back to finding" }));
    expect(within(finding).getByText(/The support agent retries/)).toBeVisible();
    await user.click(within(finding).getByRole("button", { name: "Close finding (Esc)" }));
    expect(await screen.findByRole("grid", { name: "Findings" })).toBeVisible();
    expect(screen.queryByRole("complementary", { name: "Finding details" })).not.toBeInTheDocument();
    expect(network).not.toHaveBeenCalled();
    await expectUrl(onUrlUpdate, (url) => expect(url.get("demo")).toBe("true"));
    await expectUrl(onUrlUpdate, (url) => expect(url.has("span")).toBe(false));
    await user.click(screen.getByRole("switch", { name: "Demo data" }));
    await expectUrl(onUrlUpdate, (url) => expect(url.get("demo")).toBeNull());
    expect(screen.getByRole("switch", { name: "Demo data" })).not.toBeChecked();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("has no duplicate sample buttons for existing investigations or populated traces", async () => {
    const user = userEvent.setup();
    const data = createLensDemoData();
    const saved = data.lenses[0];
    network.mockImplementation(async (input) => {
      const path = requestPath(input);
      if (path === "/lens/signals") return Response.json({ model: "", threshold: 0.5, signals: [] });
      if (path === "/lens")
        return Response.json({
          lenses: [saved],
          workers: [],
          tracing_enabled: true,
        });
      if (path.endsWith("/reviews")) return Response.json({ reviews: [], reviewed: saved.jobs[0].reviewed });
      if (path.endsWith("/runs")) return Response.json(saved.jobs);
      if (path === "/v1/traces") return Response.json({ data: data.runs.map((run) => run.trace.summary) });
      if (path === "/lens/traces/findings") return Response.json([]);
      if (path === "/lens/feedback/summary") return Response.json([]);
      return defaultResponse(path, true);
    });
    renderWithProviders(<LensWorkspace accessToken="live-token" userRole="Admin" readOnly={false} />, {
      searchParams: `?tab=investigations&lens=${saved.id}`,
    });
    expect(await screen.findByRole("heading", { name: saved.settings.name })).toBeVisible();
    expect(screen.queryByRole("button", { name: "Preview sample" })).not.toBeInTheDocument();
    await user.click(within(screen.getByRole("tablist", { name: "Lens" })).getByRole("tab", { name: "Traces" }));
    expect(await screen.findByText("Where is order #1042?")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Preview sample" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Connect project" })).toBeEnabled();
  });

  it("uses one demo switch across setup, existing investigations, and demo traces", async () => {
    const user = userEvent.setup();
    const saved = createLensDemoData().lenses[0];
    network.mockImplementation(async (input) => {
      const path = requestPath(input);
      if (path === "/lens/signals") return Response.json({ model: "", threshold: 0.5, signals: [] });
      if (path === "/lens")
        return Response.json({
          lenses: [saved],
          workers: [],
          tracing_enabled: false,
        });
      if (path.endsWith("/reviews")) return Response.json({ reviews: [], reviewed: saved.jobs[0].reviewed });
      if (path.endsWith("/runs")) return Response.json(saved.jobs);
      if (path === "/v1/traces") return Response.json({ detail: "Tracing is not enabled" }, { status: 501 });
      return defaultResponse(path, false);
    });
    renderWithProviders(<LensWorkspace accessToken="live-token" userRole="Admin" readOnly={false} />);
    expect(await screen.findByRole("region", { name: "Get started with Lens" })).toBeVisible();
    expect(screen.getAllByRole("switch", { name: "Demo data" })).toHaveLength(1);
    expect(screen.queryByRole("button", { name: /Preview sample|Explore with sample data/ })).not.toBeInTheDocument();
    const tabs = within(screen.getByRole("tablist", { name: "Lens" }));
    await user.click(tabs.getByRole("tab", { name: "Investigations" }));
    expect(await screen.findByRole("row", { name: new RegExp(saved.settings.name) })).toBeVisible();
    expect(screen.queryByRole("button", { name: "Preview sample" })).not.toBeInTheDocument();
    await user.click(tabs.getByRole("tab", { name: "Traces" }));
    await user.click(screen.getByRole("switch", { name: "Demo data" }));
    expect(await screen.findByRole("table", { name: "Agent runs" })).toBeVisible();
    expect(screen.queryByRole("button", { name: "Preview sample" })).not.toBeInTheDocument();
  });

  it("marks the Investigations tab while a scan runs and clears it once the scan finishes", async () => {
    const saved = createLensDemoData().lenses[0];
    const withJob = (status: (typeof saved.jobs)[number]["status"]) => ({
      ...saved,
      jobs: [{ ...saved.jobs[0], status }, ...saved.jobs.slice(1)],
    });
    const lenses = vi.fn(() => [withJob("running")]);
    network.mockImplementation(async (input) => {
      const path = requestPath(input);
      if (path === "/lens/signals") return Response.json({ model: "", threshold: 0.5, signals: [] });
      if (path === "/lens")
        return Response.json({
          lenses: lenses(),
          workers: [],
          tracing_enabled: false,
        });
      if (path === "/v1/traces") return Response.json({ detail: "Tracing is not enabled" }, { status: 501 });
      return defaultResponse(path, false);
    });
    renderWithProviders(<LensWorkspace accessToken="live-token" userRole="Admin" readOnly={false} />);
    const tab = within(screen.getByRole("tablist", { name: "Lens" })).getByRole("tab", { name: "Investigations" });
    await waitFor(() => expect(tab).toHaveAccessibleDescription("An investigation is running"));
    lenses.mockReturnValue([withJob("completed")]);
    await testQueryClient.refetchQueries({ queryKey: lensKeys.lists() });
    await waitFor(() => expect(tab).toHaveAccessibleDescription(""));
  });

  it("keeps the Investigations tab selected while editing and returns to its list", async () => {
    const user = userEvent.setup();
    const saved = createLensDemoData().lenses[0];
    network.mockImplementation(async (input) => {
      const path = requestPath(input);
      if (path === "/lens/signals") return Response.json({ model: "", threshold: 0.5, signals: [] });
      if (path === "/lens")
        return Response.json({
          lenses: [saved],
          workers: [],
          tracing_enabled: true,
        });
      if (path.endsWith("/reviews")) return Response.json({ reviews: [], reviewed: saved.jobs[0].reviewed });
      if (path.endsWith("/runs")) return Response.json(saved.jobs);
      if (path === "/lens/agents") return Response.json([]);
      if (path.startsWith("/lens/preview")) return Response.json({ eligible: 0, selected: 0, executions: [] });
      if (path === "/v1/traces") return Response.json({ detail: "Tracing is not enabled" }, { status: 501 });
      return defaultResponse(path, true);
    });
    renderWithProviders(<LensWorkspace accessToken="live-token" userRole="Admin" readOnly={false} />, {
      searchParams: "?tab=investigations&dialog=new",
    });
    const tabs = within(screen.getByRole("tablist", { name: "Lens" }));
    expect(await screen.findByRole("region", { name: "New investigation" })).toBeVisible();
    const tab = tabs.getByRole("tab", { name: /^Investigations/ });
    expect(tab).toHaveAttribute("aria-selected", "true");
    expect(await screen.findByRole("region", { name: "New investigation" })).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Back to investigations" }));
    expect(await screen.findByRole("row", { name: new RegExp(saved.settings.name) })).toBeVisible();
    expect(await screen.findByRole("table", { name: "Investigations" })).toBeVisible();
  });

  it.each(["standalone", "embedded"] as const)("shows deployment analysis and worker health on %s", async (surface) => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    const saved = createLensDemoData().lenses[0];
    const worker = {
      id: "worker",
      name: "Worker",
      revoked: false,
      analysis_key_id: "a".repeat(64),
      scope: saved.scope,
      last_seen: new Date().toISOString(),
    };
    const workers = vi.fn(() => [worker]);
    network.mockImplementation(async (input) => {
      const path = requestPath(input);
      if (path === "/lens/model_group/info")
        return Response.json({
          data: [{ model_group: "analysis", providers: ["OpenAI"], mode: "chat" }],
        });
      if (path === "/lens/signals") return Response.json({ model: "", threshold: 0.5, signals: [] });
      if (path === "/lens")
        return Response.json({
          lenses: [saved],
          workers: workers(),
          tracing_enabled: true,
        });
      if (path.endsWith("/reviews")) return Response.json({ reviews: [], reviewed: saved.jobs[0].reviewed });
      if (path.endsWith("/runs")) return Response.json(saved.jobs);
      if (path === "/v1/traces") return Response.json({ detail: "Tracing is not enabled" }, { status: 501 });
      return defaultResponse(path, true);
    });
    renderWithProviders(
      <LensHostProvider host={{ surface }}>
        <LensWorkspace accessToken="live-token" userRole="Admin" readOnly={false} />
      </LensHostProvider>,
      { onUrlUpdate },
    );
    const tabs = within(screen.getByRole("tablist", { name: "Lens" }));
    const settings = await tabs.findByRole("tab", { name: "Settings" });
    expect(settings).toHaveAttribute("title", "Analysis configured");
    await user.click(settings);
    await expectUrl(onUrlUpdate, (url) => expect(url.get("tab")).toBe("settings"));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    const panel = within(screen.getByRole("region", { name: "Settings" }));
    expect(panel.getByText("Tracing enabled", { exact: true })).toBeVisible();
    expect(panel.getByRole("heading", { name: "Analysis", exact: true })).toBeVisible();
    expect(await panel.findByText("Analysis is configured")).toBeVisible();
    expect(panel.getByRole("list", { name: "Analysis models" })).toHaveTextContent("analysis · OpenAI");
    expect(panel.getByRole("button", { name: "Check configuration" })).toBeEnabled();
    workers.mockReturnValue([{ ...worker, revoked: true }]);
    await testQueryClient.refetchQueries({ queryKey: lensKeys.lists() });
    await waitFor(() => expect(settings).toHaveAttribute("title", "Configure analysis"));
    expect(await panel.findByText("Models are configured; waiting for the investigation worker")).toBeVisible();
    await user.click(panel.getByRole("button", { name: "Connect project" }));
    await expectUrl(onUrlUpdate, (url) => expect(url.get("tab")).toBe("home"));
    expect(tabs.getByRole("tab", { name: "Home" })).toHaveAttribute("aria-selected", "true");
    expect(await screen.findByRole("button", { name: "Copy setup instructions" })).toBeVisible();
  });

  it("sends the first-time guide's Configure analysis into the Settings tab", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    network.mockImplementation(async (input) => {
      const path = requestPath(input);
      if (path === "/lens/signals") return Response.json({ model: "", threshold: 0.5, signals: [] });
      if (path === "/lens")
        return Response.json({
          lenses: [],
          workers: [],
          tracing_enabled: true,
        });
      if (path === "/v1/traces") return Response.json({ data: [{}] });
      return defaultResponse(path, true);
    });
    renderWithProviders(<LensWorkspace accessToken="live-token" userRole="Admin" readOnly={false} />, {
      searchParams: "?tab=investigations",
      onUrlUpdate,
    });
    const guide = within(await screen.findByRole("region", { name: "Get Lens running" }));
    await user.click(await guide.findByRole("button", { name: "Configure analysis" }));
    await expectUrl(onUrlUpdate, (url) => expect(url.get("tab")).toBe("settings"));
    const panel = within(await screen.findByRole("region", { name: "Settings" }));
    expect(await panel.findByRole("heading", { name: "Add an analysis provider" })).toBeVisible();
    expect(panel.getByRole("link", { name: "Configure analysis models" })).toHaveAttribute(
      "href",
      "https://github.com/BerriAI/lens/blob/main/docs/analysis.md",
    );
  });

  it("resumes deployment setup across tab switches and offers the first investigation when ready", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    const worker = {
      id: "worker",
      name: "Lens worker",
      revoked: false,
      analysis_key_id: "b".repeat(64),
      scope: { all_teams: true, api_key_hash: "", team_id: "" },
      last_seen: "1970-01-01T00:00:00Z",
    };
    const workers = vi.fn((): (typeof worker)[] => []);
    const models = vi.fn((): { model_group: string; providers: string[]; mode: string }[] => []);
    network.mockImplementation(async (input) => {
      const path = requestPath(input);
      if (path === "/lens/signals") return Response.json({ model: "", threshold: 0.5, signals: [] });
      if (path === "/lens")
        return Response.json({
          lenses: [],
          workers: workers(),
          tracing_enabled: true,
        });
      if (path === "/lens/model_group/info") return Response.json({ data: models() });
      if (path === "/lens/models")
        return Response.json({
          data: models().map(({ model_group }) => ({ id: model_group })),
        });
      if (path === "/lens/signals") return Response.json({ model: "", threshold: 0.5, signals: [] });
      if (path === "/lens/agents") return Response.json([]);
      if (path.startsWith("/lens/preview")) return Response.json({ eligible: 0, selected: 0, executions: [] });
      if (path === "/v1/traces") return Response.json({ data: [createLensDemoData().runs[0].trace.summary] });
      if (path === "/lens/traces/findings") return Response.json([]);
      if (path === "/lens/feedback/summary") return Response.json([]);
      return defaultResponse(path, true);
    });
    renderWithProviders(<LensWorkspace accessToken="live-token" userRole="Admin" readOnly={false} />, {
      searchParams: "?tab=settings",
      onUrlUpdate,
    });
    const panel = within(await screen.findByRole("region", { name: "Settings" }));
    expect(await panel.findByRole("heading", { name: "Add an analysis provider" })).toBeVisible();
    expect(panel.queryByRole("button", { name: "New investigation" })).not.toBeInTheDocument();
    models.mockReturnValue([{ model_group: "analysis", providers: ["OpenAI"], mode: "chat" }]);
    workers.mockReturnValue([worker]);
    await user.click(panel.getByRole("button", { name: "Check configuration" }));
    expect(await panel.findByText("Models are configured; waiting for the investigation worker")).toBeVisible();
    expect(panel.queryByRole("button", { name: "New investigation" })).not.toBeInTheDocument();
    const tabs = within(screen.getByRole("tablist", { name: "Lens" }));
    await user.click(tabs.getByRole("tab", { name: "Traces" }));
    expect(await screen.findByRole("table", { name: "Agent runs" })).toBeVisible();
    await user.click(tabs.getByRole("tab", { name: "Settings" }));
    expect(panel.getByRole("list", { name: "Analysis models" })).toHaveTextContent("analysis · OpenAI");
    workers.mockReturnValue([{ ...worker, last_seen: new Date().toISOString() }]);
    await user.click(panel.getByRole("button", { name: "Check configuration" }));
    expect(await panel.findByText("Analysis is configured")).toBeVisible();
    expect(network.mock.calls.every(([input]) => !requestPath(input).startsWith("/key/"))).toBe(true);
    expect(network.mock.calls.every(([input]) => !requestPath(input).startsWith("/lens/workers/"))).toBe(true);
    await user.click(panel.getByRole("button", { name: "New investigation" }));
    await expectUrl(onUrlUpdate, (url) => expect(url.get("tab")).toBe("investigations"));
    expect(lastUrl(onUrlUpdate).get("dialog")).toBe("new");
    expect(await screen.findByRole("region", { name: "New investigation" })).toBeVisible();
  });

  it.each(["standalone", "embedded"] as const)("hides Settings for read-only %s sessions", async (surface) => {
    renderWithProviders(
      <LensHostProvider host={{ surface }}>
        <LensWorkspace accessToken="live-token" userRole="Admin" readOnly />
      </LensHostProvider>,
    );
    expect(await screen.findByRole("tablist", { name: "Lens" })).toBeVisible();
    await waitFor(() => expect(network).toHaveBeenCalled());
    expect(screen.queryByRole("tab", { name: "Settings" })).not.toBeInTheDocument();
  });

  it("sends read-only sessions following a Settings link to the default tab", async () => {
    renderWithProviders(<LensWorkspace accessToken="live-token" userRole="Admin" readOnly />, {
      searchParams: "?tab=settings",
    });
    expect(await screen.findByRole("tab", { name: "Home", selected: true })).toBeVisible();
  });

  it("turns the worker health dot off once heartbeats expire even when polling returns unchanged data", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const saved = createLensDemoData().lenses[0];
      const worker = {
        id: "worker",
        name: "Worker",
        revoked: false,
        analysis_key_id: "a".repeat(64),
        scope: saved.scope,
        last_seen: new Date().toISOString(),
      };
      network.mockImplementation(async (input) => {
        const path = requestPath(input);
        if (path === "/lens/signals") return Response.json({ model: "", threshold: 0.5, signals: [] });
        if (path === "/lens")
          return Response.json({
            lenses: [saved],
            workers: [worker],
            tracing_enabled: true,
          });
        if (path === "/v1/traces") return Response.json({ detail: "Tracing is not enabled" }, { status: 501 });
        return defaultResponse(path, true);
      });
      renderWithProviders(<LensWorkspace accessToken="live-token" userRole="Admin" readOnly={false} />);
      const tabs = within(screen.getByRole("tablist", { name: "Lens" }));
      const settings = await tabs.findByRole("tab", { name: "Settings" });
      expect(settings).toHaveAttribute("title", "Analysis configured");
      await vi.advanceTimersByTimeAsync(130000);
      await waitFor(() => expect(settings).toHaveAttribute("title", "Configure analysis"));
    } finally {
      vi.useRealTimers();
    }
  });

  it("polls /lens every 2s while Settings waits for a worker, then drops to the 10s cadence once it connects", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const saved = createLensDemoData().lenses[0];
      const worker = {
        id: "worker",
        name: "Worker",
        revoked: false,
        analysis_key_id: "a".repeat(64),
        scope: saved.scope,
        last_seen: new Date(Date.now() - 600_000).toISOString(),
      };
      const workers = vi.fn(() => [worker]);
      const listCalls = () => network.mock.calls.filter(([input]) => requestPath(input) === "/lens").length;
      network.mockImplementation(async (input) => {
        const path = requestPath(input);
        if (path === "/lens/signals") return Response.json({ model: "", threshold: 0.5, signals: [] });
        if (path === "/lens")
          return Response.json({
            lenses: [saved],
            workers: workers(),
            tracing_enabled: true,
          });
        if (path === "/v1/traces") return Response.json({ detail: "Tracing is not enabled" }, { status: 501 });
        return defaultResponse(path, true);
      });
      renderWithProviders(<LensWorkspace accessToken="live-token" userRole="Admin" readOnly={false} />, {
        searchParams: "?tab=settings",
      });
      expect(await screen.findByRole("region", { name: "Settings" })).toBeVisible();
      const initial = listCalls();
      await vi.advanceTimersByTimeAsync(2000);
      await waitFor(() => expect(listCalls()).toBe(initial + 1));
      workers.mockReturnValue([{ ...worker, last_seen: new Date().toISOString() }]);
      await vi.advanceTimersByTimeAsync(2000);
      await waitFor(() => expect(listCalls()).toBe(initial + 2));
      await vi.advanceTimersByTimeAsync(2000);
      expect(listCalls()).toBe(initial + 2);
      await vi.advanceTimersByTimeAsync(8000);
      await waitFor(() => expect(listCalls()).toBe(initial + 3));
    } finally {
      vi.useRealTimers();
    }
  });
});

it("keeps demo row actions visible and opens reviewed traces without touching live data", async () => {
  const user = userEvent.setup();
  renderWithProviders(<LensWorkspace accessToken="live-token" userRole="Admin" readOnly={false} />, {
    searchParams: "?tab=investigations&demo=true",
  });
  const row = await screen.findByRole("row", { name: "Support quality" });
  expect(within(row).getByRole("button", { name: "Run Support quality now" })).toBeDisabled();
  expect(within(row).getByRole("button", { name: "Edit Support quality" })).toBeDisabled();
  await user.click(row);
  await user.click(await screen.findByRole("button", { name: "View run" }));
  const reviews = await screen.findByRole("list", { name: "Reviewed traces" });
  expect(within(reviews).getAllByRole("button").length).toBeGreaterThan(0);
  expect(screen.getByRole("region", { name: "Preliminary observations" })).toHaveTextContent("Repeated lookups");
  expect(network).not.toHaveBeenCalled();
});

it("keeps trace quick filters in links and clears them when leaving demo data", async () => {
  const user = userEvent.setup();
  const onUrlUpdate = vi.fn();
  renderWithProviders(<LensWorkspace accessToken="live-token" userRole="Admin" readOnly={false} />, {
    searchParams: "?tab=traces&demo=true&agent=support_agent&status=error",
    onUrlUpdate,
  });
  const table = await screen.findByRole("table", { name: "Agent runs" });
  await waitFor(() => expect(within(table).getAllByRole("row")).toHaveLength(4));
  await user.click(screen.getByRole("combobox", { name: "Filter traces by status" }));
  await user.click(await screen.findByRole("option", { name: "No errors" }));
  await expectUrl(onUrlUpdate, (url) => expect(url.get("status")).toBe("ok"));
  expect(await within(table).findByText("Can I return my headphones?")).toBeVisible();
  expect(within(table).queryByText("Where is order #1042?")).not.toBeInTheDocument();
  await user.click(screen.getByRole("switch", { name: "Demo data" }));
  await expectUrl(onUrlUpdate, (url) => expect([...url.keys()]).toEqual(["tab"]));
});

describe("Lens agent selector", () => {
  it("should keep picker keyboard focus, switch agents from the sidebar, and reopen the pick after a refresh", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    const first = renderWithProviders(<LensWorkspace accessToken="live-token" userRole="Admin" readOnly={false} />, {
      searchParams: "?demo=true",
      onUrlUpdate,
    });
    const picker = await screen.findByRole("button", { name: "Agent: support_agent" });
    const runs = await screen.findByRole("table", { name: "Agent runs" });
    expect(await within(runs).findByText("Where is order #1042?")).toBeVisible();
    expect(screen.queryByRole("combobox", { name: "Filter traces by agent" })).not.toBeInTheDocument();

    await user.click(picker);
    const search = screen.getByRole("textbox", { name: "Find agent" });
    await user.click(search);
    expect(search).toHaveFocus();
    await user.keyboard("{ArrowDown}");
    expect(search).toHaveFocus();
    await user.type(search, "release");
    const options = screen.getByRole("list", { name: "Agents" });
    expect(
      within(options)
        .getAllByRole("button")
        .map((button) => button.textContent),
    ).toEqual([expect.stringContaining("release_agent")]);
    await user.click(within(options).getByRole("button", { name: /release_agent/ }));
    expect(await screen.findByRole("button", { name: "Agent: release_agent" })).toBeVisible();
    await waitFor(() => expect(within(runs).queryByText("Where is order #1042?")).not.toBeInTheDocument());
    await expectUrl(onUrlUpdate, (url) => expect(url.get("agent")).toBe("release_agent"));
    expect(window.localStorage.getItem("litellm.lens.agent.demo")).toBe("release_agent");
    expect(window.localStorage.getItem("litellm.lens.agent")).toBeNull();

    first.unmount();
    renderWithProviders(<LensWorkspace accessToken="live-token" userRole="Admin" readOnly={false} />, {
      searchParams: "?demo=true",
    });
    expect(await screen.findByRole("button", { name: "Agent: release_agent" })).toBeVisible();
  });

  it("lets a shared link choose the agent over the remembered one", async () => {
    window.localStorage.setItem("litellm.lens.agent.demo", "release_agent");
    renderWithProviders(<LensWorkspace accessToken="live-token" userRole="Admin" readOnly={false} />, {
      searchParams: "?demo=true&agent=research_agent",
    });
    expect(await screen.findByRole("button", { name: "Agent: research_agent" })).toBeVisible();
  });

  it("should open an agent from the directory and select its Traces navigation item", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    renderWithProviders(<LensWorkspace accessToken="live-token" userRole="Admin" readOnly={false} />, {
      searchParams: "?demo=true&tab=agents",
      onUrlUpdate,
    });
    const sidebar = within(screen.getByRole("complementary", { name: "Lens navigation" }));
    expect(sidebar.getByRole("tab", { name: "Agents" })).toHaveAttribute("aria-selected", "true");
    const directory = await screen.findByRole("region", { name: "Agents directory" });
    await user.click(await within(directory).findByRole("button", { name: "Open research_agent" }));

    expect(sidebar.getByRole("tab", { name: "Traces" })).toHaveAttribute("aria-selected", "true");
    expect(sidebar.getByRole("button", { name: "Agent: research_agent" })).toBeVisible();
    expect(await screen.findByRole("table", { name: "Agent runs" })).toBeVisible();
    expect(screen.queryByRole("region", { name: "Agents directory" })).not.toBeInTheDocument();
    await expectUrl(onUrlUpdate, (url) => expect(url.get("agent")).toBe("research_agent"));
    expect(lastUrl(onUrlUpdate).get("tab")).toBe("traces");
    expect(lastUrl(onUrlUpdate).get("demo")).toBe("true");

    await user.click(screen.getByRole("button", { name: "Agents", exact: true }));
    expect(await screen.findByRole("region", { name: "Agents directory" })).toBeVisible();
    expect(sidebar.getByRole("tab", { name: "Agents" })).toHaveAttribute("aria-selected", "true");
    expect(network).not.toHaveBeenCalled();
  });

  it("should keep navigation usable when collapsed and restore the selected agent when expanded", async () => {
    const user = userEvent.setup();
    renderWithProviders(<LensWorkspace accessToken="live-token" userRole="Admin" readOnly={false} />, {
      searchParams: "?demo=true&agent=research_agent",
    });
    const sidebar = within(screen.getByRole("complementary", { name: "Lens navigation" }));
    expect(await sidebar.findByRole("button", { name: "Agent: research_agent" })).toBeVisible();
    await user.click(sidebar.getByRole("button", { name: "Collapse navigation" }));

    expect(sidebar.getByRole("button", { name: "Expand navigation" })).toBeVisible();
    expect(sidebar.queryByRole("button", { name: "Agent: research_agent" })).not.toBeInTheDocument();
    expect(sidebar.getByRole("link", { name: "Documentation" })).toBeVisible();
    await user.click(sidebar.getByRole("tab", { name: "Investigations" }));
    expect(await screen.findByRole("table", { name: "Investigations" })).toBeVisible();
    expect(sidebar.getByRole("tab", { name: "Investigations" })).toHaveAttribute("aria-selected", "true");
    await user.click(sidebar.getByRole("tab", { name: "Agents" }));
    expect(await screen.findByRole("region", { name: "Agents directory" })).toBeVisible();

    await user.click(sidebar.getByRole("button", { name: "Expand navigation" }));
    expect(sidebar.getByRole("button", { name: "Collapse navigation" })).toBeVisible();
    expect(sidebar.getByRole("button", { name: "Agent: research_agent" })).toBeVisible();
    expect(sidebar.getByRole("tab", { name: "Agents" })).toHaveAttribute("aria-selected", "true");
  });

  it("should move focus down the vertical sidebar and activate a view only after Enter", async () => {
    const user = userEvent.setup();
    renderWithProviders(<LensWorkspace accessToken="live-token" userRole="Admin" readOnly={false} />, {
      searchParams: "?demo=true&agent=support_agent",
    });
    const tabs = screen.getByRole("tablist", { name: "Lens" });
    expect(tabs).toHaveAttribute("aria-orientation", "vertical");
    const traces = within(tabs).getByRole("tab", { name: "Traces" });
    const findings = within(tabs).getByRole("tab", { name: "Findings" });
    await user.click(traces);
    await user.keyboard("{ArrowDown}");

    expect(findings).toHaveFocus();
    expect(traces).toHaveAttribute("aria-selected", "true");
    expect(findings).toHaveAttribute("aria-selected", "false");
    await user.keyboard("{Enter}");
    expect(findings).toHaveAttribute("aria-selected", "true");
    expect(await screen.findByRole("grid", { name: "Findings" })).toBeVisible();
  });

  it.each(["standalone", "embedded"] as const)("closes an open trace when %s switches agents", async (surface) => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    renderWithProviders(
      <LensHostProvider host={{ surface }}>
        <LensWorkspace accessToken="live-token" userRole="Admin" readOnly={false} />
      </LensHostProvider>,
      { searchParams: "?demo=true&agent=support_agent", onUrlUpdate },
    );
    await user.click(await screen.findByText("Where is order #1042?"));
    const drawer = await screen.findByRole("complementary", { name: "Trace details" });
    expect(drawer).toBeVisible();
    await expectUrl(onUrlUpdate, (url) => expect(url.get("trace")).toBeTruthy());
    await user.click(screen.getByRole("button", { name: "Agent: support_agent" }));
    const picker = screen.getByRole("list", { name: "Agents" });
    await user.click(within(picker).getByRole("button", { name: /release_agent/ }));

    await waitFor(() => expect(drawer).toHaveAttribute("data-state", "closing"));
    act(() => fireEvent.animationEnd(drawer));
    expect(screen.queryByRole("complementary", { name: "Trace details" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Agent: release_agent" })).toBeVisible();
    expect(screen.getByRole("tab", { name: "Traces" })).toHaveAttribute("aria-selected", "true");
    const runs = screen.getByRole("table", { name: "Agent runs" });
    expect(within(runs).queryByText("Where is order #1042?")).not.toBeInTheDocument();
    await expectUrl(onUrlUpdate, (url) => expect(url.get("agent")).toBe("release_agent"));
    expect(lastUrl(onUrlUpdate).has("trace")).toBe(false);
    expect(lastUrl(onUrlUpdate).get("tab")).toBe("traces");
  });
});

it("uses horizontal tabs in the embedded header and restores the selected view from its URL", async () => {
  const user = userEvent.setup();
  const onUrlUpdate = vi.fn();
  const workspace = (
    <LensHostProvider host={{ surface: "embedded" }}>
      <LensWorkspace accessToken="live-token" userRole="Admin" readOnly={false} />
    </LensHostProvider>
  );
  const first = renderWithProviders(workspace, {
    searchParams: "?demo=true&agent=support_agent&tab=traces",
    onUrlUpdate,
  });
  expect(screen.getByRole("heading", { name: "Lens", level: 1 })).toBeVisible();
  expect(await screen.findByRole("button", { name: "Agent: support_agent" })).toBeVisible();
  expect(screen.queryByRole("complementary", { name: "Lens navigation" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Open navigation" })).not.toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Documentation" })).toBeVisible();
  const tabs = screen.getByRole("tablist", { name: "Lens" });
  const traces = within(tabs).getByRole("tab", { name: "Traces" });
  const findings = within(tabs).getByRole("tab", { name: "Findings" });
  await user.click(traces);
  await user.keyboard("{ArrowRight}");
  expect(findings).toHaveFocus();
  expect(traces).toHaveAttribute("aria-selected", "true");
  await user.keyboard("{Enter}");
  expect(findings).toHaveAttribute("aria-selected", "true");
  expect(await screen.findByRole("grid", { name: "Findings" })).toBeVisible();
  await expectUrl(onUrlUpdate, (url) => expect(url.get("tab")).toBe("findings"));
  const searchParams = lastUrl(onUrlUpdate);
  expect(searchParams.get("agent")).toBe("support_agent");
  expect(searchParams.get("demo")).toBe("true");
  first.unmount();
  renderWithProviders(workspace, { searchParams });
  expect(screen.getByRole("tab", { name: "Findings" })).toHaveAttribute("aria-selected", "true");
  expect(await screen.findByRole("grid", { name: "Findings" })).toBeVisible();
  expect(network).not.toHaveBeenCalled();
});

it("should switch color themes from the sidebar and restore the saved preference after reopening", async () => {
  const user = userEvent.setup();
  const initialClassName = document.documentElement.className;
  const initialColorScheme = document.documentElement.style.colorScheme;
  const storageKey = "lens-theme-test";
  const workspace = (
    <ThemeProvider attribute="class" defaultTheme="light" storageKey={storageKey}>
      <LensWorkspace accessToken="live-token" userRole="Admin" readOnly={false} />
    </ThemeProvider>
  );

  try {
    const first = renderWithProviders(workspace, {
      searchParams: "?demo=true",
    });
    expect(document.documentElement).toHaveClass("light");
    expect(screen.getByRole("button", { name: "Light mode" })).toHaveAttribute("aria-pressed", "true");
    await user.click(screen.getByRole("button", { name: "Dark mode" }));
    expect(document.documentElement).toHaveClass("dark");
    expect(screen.getByRole("button", { name: "Dark mode" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: "Light mode" })).toHaveAttribute("aria-pressed", "false");
    expect(window.localStorage.getItem(storageKey)).toBe("dark");

    first.unmount();
    document.documentElement.classList.remove("dark");
    const reopened = renderWithProviders(workspace, {
      searchParams: "?demo=true",
    });
    expect(document.documentElement).toHaveClass("dark");
    expect(screen.getByRole("button", { name: "Dark mode" })).toHaveAttribute("aria-pressed", "true");
    await user.click(screen.getByRole("button", { name: "Light mode" }));
    expect(document.documentElement).toHaveClass("light");
    expect(document.documentElement).not.toHaveClass("dark");
    expect(screen.getByRole("button", { name: "Light mode" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: "Dark mode" })).toHaveAttribute("aria-pressed", "false");
    expect(window.localStorage.getItem(storageKey)).toBe("light");

    await user.click(screen.getByRole("button", { name: "Collapse navigation" }));
    await user.click(screen.getByRole("button", { name: "Toggle color theme" }));
    expect(document.documentElement).toHaveClass("dark");
    expect(window.localStorage.getItem(storageKey)).toBe("dark");
    await user.click(screen.getByRole("button", { name: "Expand navigation" }));
    expect(screen.getByRole("button", { name: "Dark mode" })).toHaveAttribute("aria-pressed", "true");
    reopened.unmount();
  } finally {
    document.documentElement.className = initialClassName;
    document.documentElement.style.colorScheme = initialColorScheme;
    window.localStorage.removeItem(storageKey);
  }
});
