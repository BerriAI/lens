import { beforeEach, expect, it } from "vitest";
import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderWithLens, stubGateway } from "../../../../tests/lens-test-utils";
import { testQueryClient } from "../../../../tests/test-utils";
import type { AlertRuleConfig } from "../model/types";
import { AlertRuleSettings } from "./AlertRuleSettings";

beforeEach(() => testQueryClient.clear());

it("creates separate duration and error rules through the condition picker without configuring a model", async () => {
  const gateway = stubGateway();
  gateway.get.mockReturnValue({ revision: 0, rules: [] });
  gateway.put.mockImplementation((_path, { body }) => ({ ...(body as object), revision: 1 }));
  const user = userEvent.setup();
  renderWithLens(<AlertRuleSettings />);
  await user.click(await screen.findByRole("button", { name: "Add rule" }));
  await user.click(await screen.findByRole("menuitem", { name: "Run duration" }));
  const duration = screen.getByRole("spinbutton", { name: "Run duration in minutes" });
  expect(duration).toHaveValue(5);
  fireEvent.change(duration, { target: { value: "0" } });
  expect(screen.getByRole("button", { name: "Save rules" })).toBeDisabled();
  fireEvent.change(duration, { target: { value: "12" } });
  await user.click(screen.getByRole("button", { name: "Add rule" }));
  await user.click(await screen.findByRole("menuitem", { name: "Run duration" }));
  fireEvent.change(within(screen.getByRole("group", { name: "Rule 2" })).getByRole("spinbutton"), {
    target: { value: "20" },
  });
  await user.click(screen.getByRole("button", { name: "Add rule" }));
  await user.click(await screen.findByRole("menuitem", { name: "Model error" }));
  await user.click(screen.getByRole("button", { name: "Save rules" }));
  await waitFor(() =>
    expect(gateway.put).toHaveBeenCalledWith(
      "/lens/admin/alerts/rules",
      expect.objectContaining({
        body: {
          revision: 0,
          rules: [
            { id: expect.any(String), enabled: true, condition: { kind: "run_duration", minutes: 12 } },
            { id: expect.any(String), enabled: true, condition: { kind: "run_duration", minutes: 20 } },
            { id: expect.any(String), enabled: true, condition: { kind: "model_error" } },
          ],
        },
      }),
    ),
  );
  const saved = gateway.put.mock.calls[0][1].body as AlertRuleConfig;
  expect(new Set(saved.rules.map((rule) => rule.id)).size).toBe(3);
  expect(gateway.get.mock.calls.map(([path]) => path)).toEqual(["/lens/admin/alerts/rules"]);
  expect(within(screen.getByRole("group", { name: "Rule 1" })).getByRole("spinbutton")).toHaveValue(12);
  expect(within(screen.getByRole("group", { name: "Rule 2" })).getByRole("spinbutton")).toHaveValue(20);
  expect(screen.getByRole("button", { name: "Save rules" })).toBeDisabled();
});

it("retains edits after a failed save and independently disables and removes saved rules", async () => {
  const gateway = stubGateway();
  gateway.get.mockReturnValue({
    revision: 3,
    rules: [
      { id: "duration", enabled: true, condition: { kind: "run_duration", minutes: 10 } },
      { id: "errors", enabled: true, condition: { kind: "model_error" } },
    ],
  });
  gateway.put
    .mockImplementationOnce(() => {
      throw new Error("Settings changed. Refresh and try again");
    })
    .mockImplementation((_path, { body }) => ({ ...(body as object), revision: 4 }));
  const user = userEvent.setup();
  renderWithLens(<AlertRuleSettings />);
  const first = within(await screen.findByRole("group", { name: "Rule 1" }));
  await user.click(first.getByRole("checkbox", { name: "Enabled" }));
  await user.click(screen.getByRole("button", { name: "Remove rule 2" }));
  await user.click(screen.getByRole("button", { name: "Save rules" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Settings changed");
  expect(first.getByRole("checkbox", { name: "Enabled" })).not.toBeChecked();
  expect(screen.queryByRole("group", { name: "Rule 2" })).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Save rules" }));
  await waitFor(() =>
    expect(gateway.put).toHaveBeenLastCalledWith(
      "/lens/admin/alerts/rules",
      expect.objectContaining({
        body: {
          revision: 3,
          rules: [{ id: "duration", enabled: false, condition: { kind: "run_duration", minutes: 10 } }],
        },
      }),
    ),
  );
});

it("changes the condition while preserving the saved rule identity", async () => {
  const gateway = stubGateway();
  gateway.get.mockReturnValue({
    revision: 1,
    rules: [{ id: "existing", enabled: true, condition: { kind: "run_duration", minutes: 5 } }],
  });
  gateway.put.mockImplementation((_path, { body }) => ({ ...(body as object), revision: 2 }));
  const user = userEvent.setup();
  renderWithLens(<AlertRuleSettings />);
  await user.click(await screen.findByRole("combobox", { name: "Condition" }));
  await user.click(screen.getByRole("option", { name: "Model error" }));
  expect(screen.queryByRole("spinbutton")).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Save rules" }));
  await waitFor(() =>
    expect(gateway.put).toHaveBeenCalledWith(
      "/lens/admin/alerts/rules",
      expect.objectContaining({
        body: { revision: 1, rules: [{ id: "existing", enabled: true, condition: { kind: "model_error" } }] },
      }),
    ),
  );
});
