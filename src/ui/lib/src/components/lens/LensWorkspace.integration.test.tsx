import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import {
  renderWithProviders,
  testQueryClient,
} from "../../../tests/test-utils";
import { readRequest } from "../../../tests/lens-test-utils";
import {
  admin,
  chat,
  report,
  workspaceNetwork,
} from "../../../tests/lens-workspace-utils";
import { LensWorkspace } from "./LensWorkspace";

let network: ReturnType<typeof workspaceNetwork>;
beforeEach(() => {
  testQueryClient.clear();
  window.localStorage.clear();
  window.sessionStorage.clear();
  Element.prototype.scrollIntoView = vi.fn();
  network = workspaceNetwork();
  vi.stubGlobal("fetch", network);
});

it("sends directly from Home and preserves the submitted message after an uncertain response", async () => {
  const user = userEvent.setup();
  const normal = network.getMockImplementation()!;
  const submitted = vi.fn();
  network.mockImplementation(async (input, init) => {
    const request = await readRequest(input, init);
    if (request.path.endsWith("/messages")) {
      submitted(request.body);
      if (submitted.mock.calls.length === 1)
        return Response.json(
          { detail: "Connection interrupted" },
          { status: 503 },
        );
    }
    return normal(input, init);
  });
  const onUrlUpdate = vi.fn();
  renderWithProviders(<LensWorkspace {...admin} />, { onUrlUpdate });
  expect(
    await screen.findByRole("heading", { name: "From your traces" }),
  ).toBeVisible();
  const composer = await screen.findByRole("textbox", { name: "Message Lens" });
  fireEvent.change(composer, { target: { value: "Which requests failed?" } });
  await user.click(screen.getByRole("button", { name: "Send message" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Connection interrupted",
  );
  expect(composer).toHaveValue("Which requests failed?");
  await user.click(screen.getByRole("button", { name: "Send message" }));
  await waitFor(() =>
    expect(onUrlUpdate.mock.lastCall?.[0].searchParams.get("chat")).toBe(
      chat.id,
    ),
  );
  expect(submitted).toHaveBeenCalledTimes(2);
  expect(submitted.mock.calls[0][0]).toEqual(submitted.mock.calls[1][0]);
  expect(submitted.mock.calls[0][0]).toMatchObject({
    text: "Which requests failed?",
  });
  const requests = await Promise.all(
    network.mock.calls.map(([input, init]) => readRequest(input, init)),
  );
  expect(
    requests.filter(
      ({ path, method }) =>
        path === "/lens/workspace/conversations" && method === "POST",
    ),
  ).toHaveLength(1);
});

it("opens a report and starts a conversation with its durable reference", async () => {
  const user = userEvent.setup();
  const onUrlUpdate = vi.fn();
  renderWithProviders(<LensWorkspace {...admin} />, { onUrlUpdate });
  await user.click(await screen.findByRole("button", { name: "Open report" }));
  const detail = within(
    await screen.findByRole("dialog", { name: report.title }),
  );
  expect(detail.getByText(report.markdown)).toBeVisible();
  await user.click(detail.getByRole("button", { name: "Discuss with Lens" }));
  await waitFor(() =>
    expect(onUrlUpdate.mock.lastCall?.[0].searchParams.get("chat")).toBe(
      chat.id,
    ),
  );
  const requests = await Promise.all(
    network.mock.calls.map(([input, init]) => readRequest(input, init)),
  );
  expect(
    requests.find(({ path }) => path.endsWith("/messages"))?.body,
  ).toMatchObject({ text: expect.stringContaining('report "outcomes"') });
});

it("copies all annotation pages from the feed", async () => {
  const user = userEvent.setup();
  const normal = network.getMockImplementation()!;
  const pages = vi.fn();
  network.mockImplementation(async (input, init) => {
    const request = await readRequest(input, init);
    if (request.path === "/lens/workspace/annotations/query") {
      pages(request.body);
      return Response.json(
        pages.mock.calls.length === 1
          ? {
              annotations: [
                { evidence: "First judgment", definition_key: "task_outcome" },
              ],
              next_cursor: 10,
            }
          : {
              annotations: [
                { evidence: "Second judgment", definition_key: "task_outcome" },
              ],
              next_cursor: null,
            },
      );
    }
    return normal(input, init);
  });
  renderWithProviders(<LensWorkspace {...admin} />);
  await user.click(
    await screen.findByRole("button", { name: "Copy for agent" }),
  );
  await waitFor(() => expect(pages).toHaveBeenCalledTimes(2));
  await waitFor(async () =>
    expect(await navigator.clipboard.readText()).toContain("Second judgment"),
  );
  const copied = await navigator.clipboard.readText();
  expect(copied).toContain("First judgment");
  expect(copied).toContain("scope-1");
  expect(copied).toContain(report.markdown);
  expect(pages.mock.calls[1][0]).toMatchObject({
    after: 10,
    trace_id: "trace-1",
    trace_ref: "scope-1",
  });
});

it("keeps the chosen model in the new session request", async () => {
  const user = userEvent.setup();
  renderWithProviders(<LensWorkspace {...admin} />);
  await user.click(await screen.findByRole("combobox", { name: "Model" }));
  await user.type(
    await screen.findByRole("combobox", { name: "Search models" }),
    "second",
  );
  await user.click(await screen.findByRole("option", { name: "second-model" }));
  fireEvent.change(screen.getByRole("textbox", { name: "Message Lens" }), {
    target: { value: "Find failed requests" },
  });
  await user.click(screen.getByRole("button", { name: "Send message" }));
  await waitFor(() =>
    expect(screen.getByRole("textbox", { name: "Message Lens" })).toHaveValue(
      "",
    ),
  );
  const requests = await Promise.all(
    network.mock.calls.map(([input, init]) => readRequest(input, init)),
  );
  expect(
    requests.find(
      ({ path, method }) =>
        path === "/lens/workspace/conversations" && method === "POST",
    )?.body,
  ).toMatchObject({ model: "second-model" });
});

it("waits for a configured model and resumes the draft after models are refreshed", async () => {
  const user = userEvent.setup();
  const normal = network.getMockImplementation()!;
  const models = vi
    .fn()
    .mockReturnValueOnce([])
    .mockReturnValue(["connected-model"]);
  network.mockImplementation(async (input, init) => {
    const request = await readRequest(input, init);
    if (request.path === "/lens/workspace/models")
      return Response.json(models());
    return normal(input, init);
  });
  renderWithProviders(<LensWorkspace {...admin} />);
  const refresh = await screen.findByRole("button", { name: "Refresh models" });
  const composer = screen.getByRole("textbox", { name: "Message Lens" });
  fireEvent.change(composer, { target: { value: "Find failed requests" } });
  expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
  await user.click(refresh);
  expect(
    await screen.findByRole("combobox", { name: "Model" }),
  ).toHaveTextContent("connected-model");
  expect(composer).toHaveValue("Find failed requests");
  await user.click(screen.getByRole("button", { name: "Send message" }));
  await waitFor(() =>
    expect(screen.getByRole("textbox", { name: "Message Lens" })).toHaveValue(""),
  );
  const requests = await Promise.all(
    network.mock.calls.map(([input, init]) => readRequest(input, init)),
  );
  expect(
    requests.find(
      ({ path, method }) =>
        path === "/lens/workspace/conversations" && method === "POST",
    )?.body,
  ).toMatchObject({ model: "connected-model" });
});

it("opens tagging activity without a report and queries the exact producer, definition and value", async () => {
  const user = userEvent.setup();
  const normal = network.getMockImplementation()!;
  network.mockImplementation(async (input, init) => {
    const request = await readRequest(input, init);
    if (request.path === "/lens/workspace/reports") return Response.json([]);
    if (request.path === "/lens/workspace/activity")
      return Response.json([
        {
          producer_id: "bot-a",
          producer_name: "Failure reviewer",
          definition_id: "outcome",
          definition_key: "task_outcome",
          value: "failed",
          count: 3,
          updated_at: 1,
        },
      ]);
    if (request.path === "/lens/workspace/annotations/query")
      return Response.json({
        annotations: [
          {
            trace: { trace_id: "failed-trace", trace_ref: "scope-a" },
            producer_id: "bot-a",
            producer_name: "Failure reviewer",
            definition_id: "outcome",
            definition_key: "task_outcome",
            value: "failed",
            evidence: "The requested checkout never completed",
            updated_at: 1,
          },
        ],
        next_cursor: null,
      });
    return normal(input, init);
  });
  renderWithProviders(<LensWorkspace {...admin} />);
  await user.click(
    await screen.findByRole("button", {
      name: /Failure reviewer tagged 3 traces/,
    }),
  );
  const panel = within(
    await screen.findByRole("dialog", { name: "task_outcome: failed" }),
  );
  expect(
    panel.getByText("The requested checkout never completed"),
  ).toBeVisible();
  expect(panel.getByRole("button", { name: "failed-trace" })).toBeVisible();
  const requests = await Promise.all(
    network.mock.calls.map(([input, init]) => readRequest(input, init)),
  );
  expect(
    requests.find(({ path }) => path === "/lens/workspace/annotations/query")
      ?.body,
  ).toMatchObject({
    producer_id: "bot-a",
    definition_id: "outcome",
    value: "failed",
  });
});

it("creates a bot with the selected model from the sidebar plus", async () => {
  const user = userEvent.setup();
  const normal = network.getMockImplementation()!;
  const created = vi.fn();
  network.mockImplementation(async (input, init) => {
    const request = await readRequest(input, init);
    if (request.path === "/lens/workspace/bots") {
      created(request.body);
      return Response.json({ id: "created-bot" });
    }
    return normal(input, init);
  });
  renderWithProviders(<LensWorkspace {...admin} />);
  await user.click(
    await screen.findByRole("button", { name: "Bots", exact: true }),
  );
  await user.click(await screen.findByRole("button", { name: "Create bot" }));
  const dialog = within(screen.getByRole("dialog", { name: "Create bot" }));
  fireEvent.change(dialog.getByRole("textbox", { name: "Name" }), {
    target: { value: "Outcome reviewer" },
  });
  fireEvent.change(dialog.getByRole("textbox", { name: "Task" }), {
    target: { value: "Tag unsuccessful requests" },
  });
  await user.click(dialog.getByRole("combobox", { name: "Model" }));
  await user.click(await screen.findByRole("option", { name: "second-model" }));
  await user.click(dialog.getByRole("button", { name: "Create bot" }));
  await waitFor(() =>
    expect(created).toHaveBeenCalledWith(
      expect.objectContaining({
        name: "Outcome reviewer",
        model: "second-model",
      }),
    ),
  );
});
