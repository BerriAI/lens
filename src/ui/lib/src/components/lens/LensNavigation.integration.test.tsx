import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderWithProviders, testQueryClient } from "../../../tests/test-utils";
import { requestPath } from "../../../tests/lens-test-utils";
import { LensWorkspace } from "./LensWorkspace";
import { LensHostProvider } from "../../host/LensHost";
import { DEFAULT_SIGNAL_THRESHOLD } from "./model/signals";

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

  it("offers a founder call from expanded and collapsed navigation", async () => {
    const user = userEvent.setup();
    renderWithProviders(<LensWorkspace {...admin} />, {
      searchParams: "?tab=traces",
    });
    const booking = screen.getByRole("link", { name: "Book a call" });
    expect(booking).toHaveAttribute(
      "href",
      "https://calendly.com/d/dtr8-nnr-3yy/litellm-product-team-feedback",
    );
    expect(booking).toHaveAttribute("target", "_blank");
    expect(booking).toHaveAttribute("rel", "noopener noreferrer");
    await user.hover(booking);
    expect(
      await screen.findByText("Book a call with LiteLLM founders"),
    ).toBeVisible();
    await user.unhover(booking);
    await user.click(
      screen.getByRole("button", { name: "Collapse navigation" }),
    );
    expect(screen.getByRole("link", { name: "Book a call" })).toBeVisible();
    await user.hover(screen.getByRole("link", { name: "Book a call" }));
    expect(
      await screen.findByText("Book a call with LiteLLM founders"),
    ).toBeVisible();
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

  it("filters matches, links to the selected trace and Signal, and keeps configuration and Slack together", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    const at = "2026-11-20T04:00:00Z";
    const flag = { signal_id: "timeout", name: "Provider timeout", score: 0.9 };
    const other = { signal_id: "refusal", name: "Wrong refusal", score: 0.8 };
    const alerts = {
      since: at, limit: 500, truncated: true, evaluated: 2, failed: 1, partial: 1,
      matches: [
        { trace_id: "timed-out-trace", trace_ref: "project-a", classified_at: at, flags: [flag] },
        { trace_id: "refused-trace", trace_ref: "project-b", classified_at: at, flags: [other, flag] },
      ],
    };
    const original = vi.mocked(fetch).getMockImplementation()!;
    vi.mocked(fetch).mockImplementation((input, init) => {
      const path = requestPath(input);
      if (path === "/lens/signals/overview") return Promise.resolve(Response.json(alerts));
      if (path === "/lens/traces/signals") return Promise.resolve(Response.json([{ ...alerts.matches[0], status: "classified", flags: [{ ...flag, evidence: { quote: "The request timed out without a retry", span_id: "failed-attempt" } }] }]));
      if (path === "/lens/signals")
        return Promise.resolve(Response.json({
          model: "judge", threshold: 0.7, signals: [{ id: "timeout", name: flag.name, question: "Did an upstream attempt time out?" }],
        }));
      if (path === "/lens/signals/alerts")
        return Promise.resolve(Response.json({ connected: false, enabled: false, revision: 0, signal_ids: [] }));
      return original(input, init);
    });
    renderWithProviders(<LensWorkspace {...admin} />, { searchParams: "?tab=traces", onUrlUpdate });
    await user.click(screen.getByRole("tab", { name: "Alerts" }));
    expect(await screen.findByText("Counts cover only the latest 500 results in this window.")).toBeVisible();
    const topActions = within(screen.getByRole("group", { name: "Page actions" }));
    expect(topActions.getByRole("button", { name: "Configure signals" })).toBeVisible();
    expect(topActions.getByRole("button", { name: "Slack notifications", exact: true })).toBeVisible();
    const summary = within(screen.getByRole("region", { name: "Last 24 hours" }));
    expect(summary.getByText("3")).toBeVisible();
    expect(summary.getByText("Provider timeout")).toBeVisible();
    expect(summary.getByText("2 of 3 alerts")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Filter Provider timeout, 2 alerts" }));
    expect(screen.queryByRole("button", { name: "Open Wrong refusal in trace refused-trace" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Clear filters" }));
    const chart = within(screen.getByRole("region", { name: "Alerts over time" }));
    await user.click(chart.getAllByRole("button", { name: /: 0 alerts$/ })[0]);
    expect(await screen.findByText("No matches for these filters")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Clear filters" }));
    await user.click(screen.getByRole("button", { name: "Configure signals" }));
    expect(await screen.findByRole("spinbutton", { name: "Match threshold" })).toHaveValue(70);
    await user.click(screen.getByRole("button", { name: "Close", exact: true }));
    await user.click(screen.getByRole("button", { name: "Slack notifications", exact: true }));
    expect(await screen.findByLabelText("Slack webhook URL")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Close", exact: true }));
    expect(screen.getByRole("button", { name: "Open Wrong refusal in trace refused-trace" })).toBeVisible();
    await user.click(screen.getByRole("combobox", { name: "Filter by Signal" }));
    await user.click(await screen.findByRole("option", { name: "Provider timeout", exact: true }));
    expect(screen.queryByRole("button", { name: "Open Wrong refusal in trace refused-trace" })).not.toBeInTheDocument();
    fireEvent.change(screen.getByRole("textbox", { name: "Search matches" }), { target: { value: "missing" } });
    expect(await screen.findByText("No matches for these filters")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Clear filters" }));
    await user.click(screen.getByRole("button", { name: "Open Provider timeout in trace timed-out-trace" }));
    const evidence = within(await screen.findByRole("dialog", { name: "Alert evidence" }));
    expect(evidence.getByText("timed-out-trace")).toBeVisible();
    expect(await evidence.findByText("The request timed out without a retry")).toBeVisible();
    await user.click(evidence.getByRole("button", { name: "Open trace", exact: true }));
    await waitFor(() => expect(Object.fromEntries(onUrlUpdate.mock.lastCall?.[0].searchParams)).toMatchObject({
      tab: "traces", trace: "timed-out-trace", trace_ref: "project-a", signal: "timeout",
    }));
  });

  it("keeps demo Signal and Slack settings available without requesting live activity", async () => {
    const user = userEvent.setup();
    renderWithProviders(<LensWorkspace {...admin} />, { searchParams: "?demo=true&tab=alerts" });
    expect(await screen.findByText("No evaluated traces in this window.")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Configure signals" }));
    expect(await screen.findByRole("spinbutton", { name: "Match threshold" })).toHaveValue(DEFAULT_SIGNAL_THRESHOLD * 100);
    await user.click(screen.getByRole("button", { name: "Close", exact: true }));
    await user.click(screen.getByRole("button", { name: "Slack notifications", exact: true }));
    expect(await screen.findByLabelText("Slack webhook URL")).toBeVisible();
    expect(vi.mocked(fetch).mock.calls.some(([input]) => requestPath(input).startsWith("/lens/signals"))).toBe(false);
  });

  it("shows a read-only empty dashboard for a Gateway admin viewer", async () => {
    const original = vi.mocked(fetch).getMockImplementation()!;
    vi.mocked(fetch).mockImplementation((input, init) =>
      requestPath(input) === "/lens/signals/overview"
        ? Promise.resolve(Response.json({
            since: "2026-11-20T04:00:00Z", limit: 500, truncated: false,
            evaluated: 1, failed: 0, partial: 0, matches: [],
          }))
        : original(input, init),
    );
    renderWithProviders(
      <LensHostProvider host={{ surface: "embedded" }}>
        <LensWorkspace {...admin} userRole="Admin Viewer" readOnly />
      </LensHostProvider>,
      { searchParams: "?tab=alerts" },
    );
    expect(await screen.findByText("No Signal matches in this window.")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Configure signals" })).not.toBeInTheDocument();
    expect(screen.queryByRole("dialog", { name: "Configure signals" })).not.toBeInTheDocument();
  });

  it("does not request deployment-wide alerts when a team user follows an Alerts link", async () => {
    renderWithProviders(<LensWorkspace {...admin} userRole="Internal User" />, { searchParams: "?tab=alerts" });
    expect(await screen.findByRole("tab", { name: "Home", selected: true })).toBeVisible();
    expect(screen.queryByRole("tab", { name: "Alerts" })).not.toBeInTheDocument();
    expect(vi.mocked(fetch).mock.calls.some(([input]) => requestPath(input) === "/lens/signals/overview")).toBe(false);
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
