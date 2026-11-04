import { fireEvent, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { renderWithProviders } from "../../../../../../tests/test-utils";
import { TraceTagFilters } from "./TraceTagFilters";

it("keeps tag conditions in the URL and carries them into investigation setup", async () => {
  const user = userEvent.setup();
  const onUrlUpdate = vi.fn();
  renderWithProviders(<TraceTagFilters />, {
    searchParams: "?agent=code-agent",
    onUrlUpdate,
  });
  await user.click(screen.getByRole("button", { name: "Tags" }));
  fireEvent.change(screen.getByLabelText("Tag key"), {
    target: { value: "skill" },
  });
  fireEvent.change(screen.getByLabelText("Tag value"), {
    target: { value: "diagram" },
  });
  await user.click(screen.getByRole("button", { name: "Add tag filter" }));
  expect(screen.getByText("skill: diagram")).toBeVisible();
  await user.keyboard("{Escape}");
  await user.click(
    screen.getByRole("button", { name: "Investigate tags" }),
  );
  await waitFor(() => expect(onUrlUpdate).toHaveBeenCalled());
  const params = new URLSearchParams(
    onUrlUpdate.mock.lastCall?.[0].queryString,
  );
  expect(JSON.parse(params.get("tags") ?? "null")).toEqual([
    { key: "skill", value: "diagram" },
  ]);
  expect(params.get("agent")).toBe("code-agent");
  expect(params.get("tab")).toBe("findings");
  expect(params.get("dialog")).toBe("new");
});
