import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { renderWithLens, stubGateway } from "../../../../tests/lens-test-utils";
import { testQueryClient } from "../../../../tests/test-utils";
import { createLensDemoData } from "../data/demo/fixtures";
import { findingKey, inboxRows } from "../model/inbox";
import type { Lens } from "../model/types";
import { FindingsView } from "./FindingsView";

let proxy = stubGateway();
const data = createLensDemoData();
const support = data.lenses[0];
const twin: Lens = {
  ...support,
  id: "twin",
  settings: { ...support.settings, name: "Second support review" },
};
const lenses = [support, twin, data.lenses[1]];
const issue = support.findings.find((finding) => finding.kind === "issue")!;

beforeEach(() => {
  testQueryClient.clear();
  proxy = stubGateway();
  proxy.get.mockImplementation(async (path) =>
    path === "/lens" ? { lenses, workers: [], tracing_enabled: true } : { data: [] },
  );
});

it("deduplicates findings across investigations and applies feedback to every source", async () => {
  const user = userEvent.setup();
  const onUrlUpdate = vi.fn();
  renderWithLens(<FindingsView />, {
    searchParams: "?tab=findings",
    onUrlUpdate,
  });
  const rows = await screen.findAllByRole("row", { name: issue.title });
  expect(rows).toHaveLength(1);
  expect(
    within(rows[0]).getByTitle(`2 affected traces across ${support.settings.name}, ${twin.settings.name}`),
  ).toBeVisible();
  await user.click(rows[0]);
  const panel = await screen.findByRole("complementary", {
    name: "Finding details",
  });
  fireEvent.change(within(panel).getByRole("textbox"), {
    target: { value: "A handoff now handles failures" },
  });
  await user.click(within(panel).getByRole("button", { name: "Mark resolved" }));
  await waitFor(() => expect(proxy.patch).toHaveBeenCalledTimes(2));
  expect(proxy.patch.mock.calls.map(([path, request]) => [path, request.body])).toEqual([
    [`/lens/support/findings/${issue.id}`, { status: "resolved", reason: "A handoff now handles failures" }],
    [`/lens/twin/findings/${issue.id}`, { status: "resolved", reason: "A handoff now handles failures" }],
  ]);
  await waitFor(() => expect(new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString).has("issue")).toBe(false));
});

it("omits inferred percentages when any grouped source contains imported evidence", async () => {
  const legacy = { ...issue, id: "legacy", priority: "high" as const };
  const imported = {
    ...issue,
    id: "canonical",
    priority: "low" as const,
    merged_finding_ids: [`agent-${"a".repeat(64)}`],
  };
  proxy.get.mockImplementation(async (path) =>
    path === "/lens"
      ? {
          lenses: [
            { ...support, findings: [legacy] },
            { ...twin, findings: [imported] },
          ],
          workers: [],
          tracing_enabled: true,
        }
      : { data: [] },
  );
  const user = userEvent.setup();
  renderWithLens(<FindingsView readOnly />, { searchParams: "?tab=findings" });
  const row = await screen.findByRole("row", { name: issue.title });
  expect(row).toHaveTextContent("2 traces");
  expect(row).not.toHaveTextContent("% affected");
  await user.click(row);
  const panel = await screen.findByRole("complementary", {
    name: "Finding details",
  });
  expect(panel).toHaveTextContent("2 affected traces");
  expect(within(panel).getByRole("region", { name: "Frequency" })).toHaveTextContent("Trace rate unavailable");
});

it("restores inbox filters from a link and keeps filter changes in the URL", async () => {
  const user = userEvent.setup();
  const onUrlUpdate = vi.fn();
  renderWithLens(<FindingsView />, {
    searchParams: "?tab=findings&inbox_agent=support_agent&priority=high",
    onUrlUpdate,
  });
  expect(await screen.findByRole("row", { name: issue.title })).toBeVisible();
  expect(screen.queryByRole("row", { name: data.lenses[1].findings[0].title })).not.toBeInTheDocument();
  await user.click(screen.getByRole("combobox", { name: "Filter by priority" }));
  await user.click(await screen.findByRole("option", { name: "Low", exact: true }));
  expect(await screen.findByText("No findings match these filters.")).toBeVisible();
  await waitFor(() =>
    expect(new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString).get("priority")).toBe("low"),
  );
});

it("reopens an inbox finding from a link and navigates between findings with the keyboard", async () => {
  const user = userEvent.setup();
  const onUrlUpdate = vi.fn();
  const rows = inboxRows(lenses);
  renderWithLens(<FindingsView readOnly />, {
    searchParams: `?tab=findings&issue=${encodeURIComponent(rows[0].key)}`,
    onUrlUpdate,
  });
  const panel = await screen.findByRole("complementary", {
    name: "Finding details",
  });
  expect(within(panel).getByRole("heading", { name: rows[0].title })).toBeVisible();
  expect(within(panel).queryByRole("button", { name: "Mark resolved" })).not.toBeInTheDocument();
  await user.keyboard("j");
  expect(await within(panel).findByRole("heading", { name: rows[1].title })).toBeVisible();
  await user.keyboard("{Escape}");
  await waitFor(() => expect(new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString).has("issue")).toBe(false));
});

it("keeps imported tool-call evidence without inferring trace prevalence from an unrelated job", async () => {
  const sampled = Array.from({ length: 10 }, (_, index) => ({
    ...support.jobs[0].sample!.executions[0],
    id: `sample-${index}`,
  }));
  const finding = {
    ...issue,
    id: `agent-${"a".repeat(64)}`,
    description: "8/10 completed repository tool calls failed. Controlled UI fixture.",
    occurrences: ["sample-0", "outside"],
    investigation_runs: [],
  };
  const imported: Lens = {
    ...support,
    findings: [finding],
    jobs: [
      {
        ...support.jobs[0],
        sample: { eligible: 10, selected: 10, executions: sampled },
      },
    ],
  };
  proxy.get.mockImplementation(async (path) =>
    path === "/lens" ? { lenses: [imported], workers: [], tracing_enabled: true } : { data: [] },
  );
  renderWithLens(<FindingsView readOnly />, {
    searchParams: `?tab=findings&issue=${encodeURIComponent(findingKey(imported, finding))}`,
  });
  const panel = await screen.findByRole("complementary", {
    name: "Finding details",
  });
  expect(within(panel).getByText(finding.description)).toBeVisible();
  const frequency = within(panel).getByRole("region", { name: "Frequency" });
  expect(frequency).toHaveTextContent("Trace rate unavailable");
  expect(frequency).toHaveTextContent("2 cited affected traces");
  expect(frequency).not.toHaveTextContent("1 of 10");
  expect(within(frequency).queryByTestId("frequency-chart")).not.toBeInTheDocument();
  expect(screen.getByRole("row", { name: finding.title })).toHaveTextContent("2 traces");
  expect(screen.getByRole("row", { name: finding.title })).not.toHaveTextContent("10%");
});

it("retains feedback and the open finding when a grouped review fails", async () => {
  const user = userEvent.setup();
  proxy.patch.mockImplementation(async (path) => {
    if (path.startsWith("/lens/twin/")) throw new Error("Review could not be saved");
  });
  renderWithLens(<FindingsView />, { searchParams: "?tab=findings" });
  await user.click(await screen.findByRole("row", { name: issue.title }));
  fireEvent.change(screen.getByRole("textbox"), {
    target: { value: "Keep this feedback" },
  });
  await user.click(screen.getByRole("button", { name: "Mark resolved" }));
  expect(await screen.findByText("Review could not be saved")).toBeVisible();
  expect(screen.getByRole("textbox")).toHaveValue("Keep this feedback");
  expect(screen.getByRole("button", { name: "Mark resolved" })).toBeEnabled();
});

it("reviews only the selected check when two findings have the same title", async () => {
  const other = {
    ...issue,
    id: "different-check",
    check_id: "different-check",
  };
  proxy.get.mockImplementation(async (path) =>
    path === "/lens"
      ? {
          lenses: [{ ...support, findings: [issue, other] }],
          workers: [],
          tracing_enabled: true,
        }
      : { data: [] },
  );
  const user = userEvent.setup();
  renderWithLens(<FindingsView />, { searchParams: "?tab=findings" });
  const rows = await screen.findAllByRole("row", { name: issue.title });
  expect(rows).toHaveLength(2);
  expect(within(rows[0]).getByTitle(new RegExp(`across ${support.settings.name}$`))).toBeVisible();
  await user.click(rows[0]);
  await user.click(await screen.findByRole("button", { name: "Mark resolved" }));
  await waitFor(() => expect(proxy.patch).toHaveBeenCalledTimes(1));
  expect(proxy.patch.mock.calls[0][0]).toBe(`/lens/support/findings/${issue.id}`);
});

it("opens a grouped finding from a link to any of its owning investigations", async () => {
  renderWithLens(<FindingsView readOnly />, {
    searchParams: `?tab=findings&issue=${encodeURIComponent(findingKey(twin, issue))}`,
  });
  const panel = await screen.findByRole("complementary", {
    name: "Finding details",
  });
  expect(within(panel).getByRole("heading", { name: issue.title })).toBeVisible();
  expect(screen.getByRole("row", { name: issue.title })).toHaveAttribute("aria-selected", "true");
});

it("ranks findings under high, medium and low priority headings with the highest first", async () => {
  const at = (id: string, priority: "high" | "medium" | "low", last_seen: string) => ({
    ...issue,
    id,
    check_id: id,
    title: `${priority} ${id}`,
    priority,
    last_seen,
  });
  const findings = [
    at("newest-low", "low", "2026-10-06T00:00:00Z"),
    at("old-high", "high", "2026-09-01T00:00:00Z"),
    at("medium", "medium", "2026-10-05T00:00:00Z"),
    at("new-high", "high", "2026-10-04T00:00:00Z"),
  ];
  proxy.get.mockImplementation(async (path) =>
    path === "/lens"
      ? {
          lenses: [{ ...support, findings }],
          workers: [],
          tracing_enabled: true,
        }
      : { data: [] },
  );
  renderWithLens(<FindingsView readOnly />, { searchParams: "?tab=findings" });
  const groups = await screen.findAllByRole("rowgroup");
  expect(groups.map((group) => group.getAttribute("aria-label"))).toEqual([
    "High priority findings",
    "Medium priority findings",
    "Low priority findings",
  ]);
  const titles = (group: HTMLElement) =>
    within(group)
      .getAllByRole("row")
      .map((row) => row.getAttribute("aria-label"))
      .filter(Boolean);
  expect(titles(groups[0])).toEqual(["high new-high", "high old-high"]);
  expect(groups[0]).toHaveTextContent(/High priority\s*2/);
  expect(titles(groups[2])).toEqual(["low newest-low"]);
});

it("should use the selected agent instead of a stale inbox filter", async () => {
  renderWithLens(<FindingsView agent="support_agent" readOnly />, {
    searchParams: "?tab=findings&agent=support_agent&inbox_agent=research_agent",
  });
  expect(await screen.findByRole("row", { name: issue.title })).toBeVisible();
  expect(screen.queryByRole("row", { name: data.lenses[1].findings[0].title })).not.toBeInTheDocument();
  expect(screen.queryByRole("combobox", { name: "Filter by agent" })).not.toBeInTheDocument();
  expect(screen.getByRole("combobox", { name: "Filter by priority" })).toBeVisible();
  expect(screen.getByText("1 finding")).toBeVisible();
});

it("should not open another agent's finding or evidence from a deep link", async () => {
  const otherLens = data.lenses[1];
  const otherFinding = otherLens.findings[0];
  const evidence = otherFinding.evidence[0];
  const query = new URLSearchParams({
    tab: "findings",
    agent: "support_agent",
    issue: findingKey(otherLens, otherFinding),
    evidence: evidence.execution_id,
    evidence_span: evidence.span_id,
  });
  renderWithLens(<FindingsView agent="support_agent" readOnly />, {
    searchParams: query,
  });
  expect(await screen.findByRole("row", { name: issue.title })).toBeVisible();
  expect(screen.queryByRole("complementary", { name: "Finding details" })).not.toBeInTheDocument();
  expect(screen.queryByText(otherFinding.description)).not.toBeInTheDocument();
  expect(screen.queryByTestId("evidence-view")).not.toBeInTheDocument();
  expect(proxy.get.mock.calls.some(([path]) => path.startsWith("/v1/traces/"))).toBe(false);
});

it("should hide the open report when the selected agent changes", async () => {
  const view = renderWithLens(<FindingsView agent="support_agent" readOnly />, {
    searchParams: `?tab=findings&issue=${encodeURIComponent(findingKey(support, issue))}`,
  });
  const panel = await screen.findByRole("complementary", {
    name: "Finding details",
  });
  expect(within(panel).getByRole("heading", { name: issue.title })).toBeVisible();
  view.rerender(<FindingsView agent="research_agent" readOnly />);
  expect(await screen.findByRole("row", { name: data.lenses[1].findings[0].title })).toBeVisible();
  expect(screen.queryByRole("complementary", { name: "Finding details" })).not.toBeInTheDocument();
  expect(screen.queryByText(issue.description)).not.toBeInTheDocument();
  expect(screen.queryByRole("row", { name: issue.title })).not.toBeInTheDocument();
  expect(screen.queryByRole("combobox", { name: "Filter by agent" })).not.toBeInTheDocument();
});

it("should ignore evidence that does not belong to the selected finding", async () => {
  const unrelated = data.lenses[1].findings[0].evidence[0];
  const query = new URLSearchParams({
    tab: "findings",
    agent: "support_agent",
    issue: findingKey(support, issue),
    evidence: unrelated.execution_id,
    evidence_span: unrelated.span_id,
  });
  renderWithLens(<FindingsView agent="support_agent" readOnly />, {
    searchParams: query,
  });
  const panel = await screen.findByRole("complementary", {
    name: "Finding details",
  });
  expect(within(panel).getByRole("heading", { name: issue.title })).toBeVisible();
  expect(within(panel).getByText(issue.description)).toBeVisible();
  expect(screen.queryByTestId("evidence-view")).not.toBeInTheDocument();
  expect(proxy.get.mock.calls.some(([path]) => path.startsWith("/v1/traces/"))).toBe(false);
});

it("should open evidence owned by the selected finding and return to its report", async () => {
  const user = userEvent.setup();
  const executionId = btoa(JSON.stringify(["requests", "", "support-request"]));
  const finding = {
    ...issue,
    occurrences: [executionId],
    evidence: [{ ...issue.evidence[0], execution_id: executionId }],
  };
  proxy.get.mockImplementation(async (path) => {
    if (path === "/lens")
      return {
        lenses: [{ ...support, findings: [finding] }],
        workers: [],
        tracing_enabled: true,
      };
    if (path.startsWith(`/lens/${support.id}/executions/`))
      return {
        parts: [
          {
            span_id: "support-request",
            content: "The original support response",
            truncated: false,
          },
        ],
      };
    return { data: [] };
  });
  renderWithLens(<FindingsView agent="support_agent" readOnly />, {
    searchParams: `?tab=findings&issue=${encodeURIComponent(findingKey(support, finding))}`,
  });
  const panel = await screen.findByRole("complementary", {
    name: "Finding details",
  });
  await user.click(within(panel).getByRole("button", { name: "View request" }));
  expect(await within(panel).findByText("The original support response")).toBeVisible();
  await user.click(within(panel).getByRole("button", { name: "Back to finding" }));
  expect(within(panel).getByRole("heading", { name: finding.title })).toBeVisible();
  expect(screen.queryByTestId("evidence-view")).not.toBeInTheDocument();
});
