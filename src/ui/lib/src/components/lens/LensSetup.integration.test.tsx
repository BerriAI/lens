import { fireEvent, screen, within } from "@testing-library/react";
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
