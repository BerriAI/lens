import { fireEvent, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  renderWithLens,
  stubGateway,
} from "../../../../../tests/lens-test-utils";
import { testQueryClient } from "../../../../../tests/test-utils";
import { LensHostProvider } from "../../../../host/LensHost";
import { TracingKeysDialog } from "./TracingKeysDialog";
import { tracingKeysQuery, type TracingKeyRecord } from "./tracingKeys";

const record: TracingKeyRecord = {
  id: "assigned-key",
  name: "User B agent",
  created_at: "2026-10-10T00:00:00Z",
  expires_at: null,
  created_by: "admin-a",
  tenant: {
    user_id: "user-b",
    team_id: "",
    org_id: "",
    api_key_hash: "digest",
  },
};
const secret = "lens-trace-new-secret";
const dialog = (surface: "standalone" | "embedded", open = true) => (
  <LensHostProvider host={{ surface }}>
    <TracingKeysDialog open={open} onOpenChange={vi.fn()} />
  </LensHostProvider>
);

beforeEach(() => testQueryClient.clear());

describe("assigned tracing keys", () => {
  it.each(["standalone", "embedded"] as const)(
    "creates a key and shows its owner in %s",
    async (surface) => {
      const user = userEvent.setup();
      const gateway = stubGateway();
      gateway.get.mockReturnValue([]);
      gateway.post.mockReturnValue({ key: secret, active: true, record });
      const view = renderWithLens(dialog(surface));
      expect(
        await screen.findByText(
          "No tracing keys yet. Create one on Home to connect an agent.",
        ),
      ).toBeVisible();
      expect(screen.getByRole("button", { name: "Create key" })).toBeDisabled();
      fireEvent.change(screen.getByRole("textbox", { name: "Key name" }), {
        target: { value: " User B agent " },
      });
      fireEvent.change(screen.getByRole("textbox", { name: "User ID" }), {
        target: { value: " user-b " },
      });
      await user.click(screen.getByRole("button", { name: "Create key" }));
      expect(await screen.findByText("Assigned to user-b")).toBeVisible();
      expect(screen.getByText("Created by admin-a")).toBeVisible();
      expect(gateway.post).toHaveBeenCalledWith(
        "/lens/tracing/keys",
        expect.objectContaining({
          body: { name: "User B agent", user_id: "user-b" },
        }),
      );
      expect(screen.getByText(secret)).toBeVisible();
      await user.click(
        screen.getByRole("button", { name: "Copy tracing key" }),
      );
      expect(await navigator.clipboard.readText()).toBe(secret);
      expect(
        screen.getByText(/Store this key in LITELLM_TRACING_KEY/),
      ).toBeVisible();
      expect(testQueryClient.getQueryData(tracingKeysQuery("test"))).toEqual([
        record,
      ]);
      view.rerender(dialog(surface, false));
      view.rerender(dialog(surface));
      expect(screen.queryByText(secret)).not.toBeInTheDocument();
      expect(screen.getByText("Assigned to user-b")).toBeVisible();
    },
  );

  it.each([
    { canMintTracingKey: false, readOnly: false },
    { canMintTracingKey: true, readOnly: true },
  ])("hides creation for restricted users (%j)", async (onboarding) => {
    const gateway = stubGateway();
    gateway.get.mockReturnValue([record]);
    renderWithLens(dialog("embedded"), { onboarding });
    expect(await screen.findByText("Assigned to user-b")).toBeVisible();
    expect(
      screen.queryByRole("region", { name: "Create key for a user" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Create key" }),
    ).not.toBeInTheDocument();
    expect(gateway.post).not.toHaveBeenCalled();
  });

  it("keeps failures retryable and rejects whitespace-only inputs", async () => {
    const user = userEvent.setup();
    const gateway = stubGateway();
    gateway.get.mockReturnValue([]);
    gateway.post.mockRejectedValue(new Error("unavailable"));
    renderWithLens(dialog("standalone"));
    fireEvent.change(screen.getByRole("textbox", { name: "Key name" }), {
      target: { value: "Agent" },
    });
    fireEvent.change(screen.getByRole("textbox", { name: "User ID" }), {
      target: { value: "  " },
    });
    expect(screen.getByRole("button", { name: "Create key" })).toBeDisabled();
    fireEvent.change(screen.getByRole("textbox", { name: "User ID" }), {
      target: { value: "user-b" },
    });
    await user.click(screen.getByRole("button", { name: "Create key" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Could not create the tracing key. Try again.",
    );
    expect(screen.getByRole("button", { name: "Create key" })).toBeEnabled();
    expect(screen.getByRole("textbox", { name: "User ID" })).toHaveValue(
      "user-b",
    );
    expect(testQueryClient.getQueryData(tracingKeysQuery("test"))).toEqual([]);
  });

  it.each(["user-b", ""])(
    "does not repeat creator attribution when it is %j",
    async (created_by) => {
      const gateway = stubGateway();
      gateway.get.mockReturnValue([{ ...record, created_by }]);
      renderWithLens(dialog("standalone"));
      expect(await screen.findByText("Assigned to user-b")).toBeVisible();
      expect(
        within(screen.getByRole("region", { name: record.name })).queryByText(
          /Created by/,
        ),
      ).not.toBeInTheDocument();
    },
  );
});
