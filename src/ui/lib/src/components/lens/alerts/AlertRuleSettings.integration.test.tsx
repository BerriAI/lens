import { beforeEach, expect, it } from "vitest";
import { fireEvent, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderWithLens, stubGateway } from "../../../../tests/lens-test-utils";
import { testQueryClient } from "../../../../tests/test-utils";
import { AlertRuleSettings } from "./AlertRuleSettings";

beforeEach(() => testQueryClient.clear());

it("enables both rules without loading or configuring a model", async () => {
  const gateway = stubGateway();
  gateway.get.mockReturnValue({ revision: 0, run_duration_minutes: null, model_errors: false });
  gateway.put.mockImplementation((_path, { body }) => ({ ...(body as object), revision: 1 }));
  const user = userEvent.setup();
  renderWithLens(<AlertRuleSettings />);
  await user.click(await screen.findByRole("checkbox", { name: "Run exceeds" }));
  const duration = screen.getByRole("spinbutton", { name: "Run duration in minutes" });
  expect(duration).toHaveValue(5);
  fireEvent.change(duration, { target: { value: "0" } });
  expect(screen.getByRole("button", { name: "Save rules" })).toBeDisabled();
  fireEvent.change(duration, { target: { value: "12" } });
  await user.click(screen.getByRole("checkbox", { name: "A model call fails" }));
  await user.click(screen.getByRole("button", { name: "Save rules" }));
  await waitFor(() =>
    expect(gateway.put).toHaveBeenCalledWith(
      "/lens/alerts/rules",
      expect.objectContaining({ body: { revision: 0, run_duration_minutes: 12, model_errors: true } }),
    ),
  );
  expect(gateway.get.mock.calls.map(([path]) => path)).toEqual(["/lens/alerts/rules"]);
  expect(screen.getByRole("spinbutton", { name: "Run duration in minutes" })).toHaveValue(12);
});

it("retains edits after a failed save and can disable a saved rule", async () => {
  const gateway = stubGateway();
  gateway.get.mockReturnValue({ revision: 3, run_duration_minutes: 10, model_errors: true });
  gateway.put
    .mockImplementationOnce(() => {
      throw new Error("Settings changed. Refresh and try again");
    })
    .mockImplementation((_path, { body }) => ({ ...(body as object), revision: 4 }));
  const user = userEvent.setup();
  renderWithLens(<AlertRuleSettings />);
  await user.click(await screen.findByRole("checkbox", { name: "Run exceeds" }));
  await user.click(screen.getByRole("button", { name: "Save rules" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Settings changed");
  expect(screen.getByRole("checkbox", { name: "Run exceeds" })).not.toBeChecked();
  await user.click(screen.getByRole("button", { name: "Save rules" }));
  await waitFor(() =>
    expect(gateway.put).toHaveBeenLastCalledWith(
      "/lens/alerts/rules",
      expect.objectContaining({ body: { revision: 3, run_duration_minutes: null, model_errors: true } }),
    ),
  );
});
