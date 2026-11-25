import { fireEvent, screen, within, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import {
  renderWithProviders,
  testQueryClient,
} from "../../../tests/test-utils";
import {
  admin,
  bot,
  workspaceNetwork,
} from "../../../tests/lens-workspace-utils";
import { readRequest } from "../../../tests/lens-test-utils";
import { LensWorkspace } from "./LensWorkspace";

beforeEach(() => {
  testQueryClient.clear();
  window.localStorage.clear();
  Element.prototype.scrollIntoView = vi.fn();
});

it("keeps trace setup accessible without replacing Home", async () => {
  const user = userEvent.setup();
  vi.stubGlobal("fetch", workspaceNetwork());
  renderWithProviders(<LensWorkspace {...admin} />);
  expect(
    await screen.findByRole("textbox", { name: "Message Lens" }),
  ).toBeEnabled();
  await user.click(screen.getByRole("button", { name: "Traces", exact: true }));
  expect(
    await screen.findByRole("region", { name: "Waiting for traces" }),
  ).toBeVisible();
  await user.click(screen.getByRole("button", { name: "Home", exact: true }));
  expect(
    await screen.findByRole("textbox", { name: "Message Lens" }),
  ).toBeEnabled();
});

it("allows non-admins to read shared reports without starting agents", async () => {
  const user = userEvent.setup();
  const network = workspaceNetwork(false);
  vi.stubGlobal("fetch", network);
  renderWithProviders(
    <LensWorkspace {...admin} userRole="Internal Viewer" readOnly />,
  );
  expect(
    await screen.findByRole("textbox", { name: "Message Lens" }),
  ).toBeDisabled();
  expect(
    await screen.findByRole("button", { name: "Open report" }),
  ).toBeEnabled();
  expect(
    screen.queryByRole("button", { name: "Discuss with Lens" }),
  ).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Bots", exact: true }));
  expect(
    await screen.findByRole("heading", { name: "Give a bot an ongoing task" }),
  ).toBeVisible();
  expect(
    screen.queryByRole("button", { name: "Create bot" }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Connect Slack" }),
  ).not.toBeInTheDocument();
  const requests = await Promise.all(
    network.mock.calls.map(([input, init]) => readRequest(input, init)),
  );
  expect(requests.filter(({ method }) => method !== "GET")).toEqual([]);
});

it("creates a bot with the standing task and model selected in the form", async () => {
  const user = userEvent.setup();
  const network = workspaceNetwork();
  const normal = network.getMockImplementation()!;
  const created = vi.fn();
  network.mockImplementation(async (input, init) => {
    const request = await readRequest(input, init);
    if (request.path === "/lens/workspace/bots") {
      created(request.body);
      return Response.json(bot);
    }
    return normal(input, init);
  });
  vi.stubGlobal("fetch", network);
  renderWithProviders(<LensWorkspace {...admin} />, {
    searchParams: "?tab=bots",
  });
  await user.click(
    (await screen.findAllByRole("button", { name: "Create bot" }))[0],
  );
  const form = within(
    await screen.findByRole("dialog", { name: "Create bot" }),
  );
  fireEvent.change(form.getByRole("textbox", { name: "Name" }), {
    target: { value: "Request reviewer" },
  });
  fireEvent.change(form.getByRole("textbox", { name: "Task" }), {
    target: { value: "Find unsuccessful user tasks and publish evidence" },
  });
  expect(form.getByText(/checks in every 10 minutes/)).toBeVisible();
  await user.click(form.getByRole("combobox", { name: "Model" }));
  await user.click(await screen.findByRole("option", { name: "second-model" }));
  await user.click(form.getByRole("button", { name: "Create bot" }));
  expect(
    await screen.findByRole("textbox", { name: "Message Request reviewer" }),
  ).toBeEnabled();
  expect(created).toHaveBeenCalledExactlyOnceWith({
    id: expect.any(String),
    name: "Request reviewer",
    instructions: "Find unsuccessful user tasks and publish evidence",
    model: "second-model",
  });
});

it("connects Slack from Home and keeps tokens out of browser storage", async () => {
  const user = userEvent.setup();
  const network = workspaceNetwork();
  const normal = network.getMockImplementation()!;
  const submitted = vi.fn();
  network.mockImplementation(async (input, init) => {
    const request = await readRequest(input, init);
    if (request.path === "/lens/workspace/slack") {
      if (request.method === "POST") submitted(request.body);
      return Response.json({
        configured: submitted.mock.calls.length > 0,
        connected: submitted.mock.calls.length > 0,
      });
    }
    return normal(input, init);
  });
  vi.stubGlobal("fetch", network);
  renderWithProviders(<LensWorkspace {...admin} />);
  await user.click(
    await screen.findByRole("button", { name: "Connect Slack" }),
  );
  const dialog = within(
    await screen.findByRole("dialog", { name: "Connect Slack" }),
  );
  expect(
    await dialog.findByRole("link", { name: "Create Slack app" }),
  ).toHaveAttribute("href", expect.stringContaining("manifest_json="));
  fireEvent.change(dialog.getByLabelText("Bot token"), {
    target: { value: "xoxb-fixture-token" },
  });
  fireEvent.change(dialog.getByLabelText("App-level token"), {
    target: { value: "xapp-fixture-token" },
  });
  await user.click(dialog.getByRole("button", { name: "Connect Slack" }));
  expect(await dialog.findByText(/Slack is connected/)).toBeVisible();
  expect(submitted).toHaveBeenCalledExactlyOnceWith({
    bot_token: "xoxb-fixture-token",
    app_token: "xapp-fixture-token",
  });
  expect(JSON.stringify(window.localStorage)).not.toContain("fixture-token");
  expect(JSON.stringify(window.sessionStorage)).not.toContain("fixture-token");
  expect(dialog.queryByLabelText("Bot token")).not.toBeInTheDocument();
});

it("keeps a failed Slack setup editable and clears credentials when closed", async () => {
  const user = userEvent.setup();
  const network = workspaceNetwork();
  const normal = network.getMockImplementation()!;
  network.mockImplementation(async (input, init) => {
    const request = await readRequest(input, init);
    if (request.path === "/lens/workspace/slack" && request.method === "POST")
      return Response.json(
        { detail: "Slack rejected the app-level token" },
        { status: 422 },
      );
    return normal(input, init);
  });
  vi.stubGlobal("fetch", network);
  renderWithProviders(<LensWorkspace {...admin} />);
  await user.click(
    await screen.findByRole("button", { name: "Connect Slack" }),
  );
  const dialog = within(
    await screen.findByRole("dialog", { name: "Connect Slack" }),
  );
  fireEvent.change(await dialog.findByLabelText("Bot token"), {
    target: { value: "xoxb-fixture-token" },
  });
  fireEvent.change(dialog.getByLabelText("App-level token"), {
    target: { value: "xapp-fixture-token" },
  });
  await user.click(dialog.getByRole("button", { name: "Connect Slack" }));
  expect(await dialog.findByRole("alert")).toHaveTextContent(
    "Slack rejected the app-level token",
  );
  expect(dialog.getByLabelText("Bot token")).toHaveValue("xoxb-fixture-token");
  await user.keyboard("{Escape}");
  await waitFor(() =>
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
  );
  await user.click(screen.getByRole("button", { name: "Connect Slack" }));
  const reopened = within(
    await screen.findByRole("dialog", { name: "Connect Slack" }),
  );
  expect(await reopened.findByLabelText("Bot token")).toHaveValue("");
});
