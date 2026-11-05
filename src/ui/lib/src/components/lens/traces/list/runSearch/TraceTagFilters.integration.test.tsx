import { fireEvent, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { renderWithProviders } from "../../../../../../tests/test-utils";
import { TraceTagFilters } from "./TraceTagFilters";
import { stubGateway } from "../../../../../../tests/lens-test-utils";

it("keeps tag conditions in the URL and carries them into investigation setup", async () => {
  const user = userEvent.setup();
  const onUrlUpdate = vi.fn();
  renderWithProviders(<TraceTagFilters />, {
    searchParams: "?agent=code-agent",
    onUrlUpdate,
  });
  await user.click(screen.getByRole("button", { name: "Tags" }));
  await user.click(screen.getByText("Filter by metadata or Lens tags"));
  fireEvent.change(screen.getByLabelText("Tag key"), {
    target: { value: "skill" },
  });
  fireEvent.change(screen.getByLabelText("Tag value"), {
    target: { value: "diagram" },
  });
  await user.click(screen.getByRole("button", { name: "Add tag filter" }));
  expect(screen.getByText("skill: diagram")).toBeVisible();
  await user.keyboard("{Escape}");
  await user.click(screen.getByRole("button", { name: "Investigate tags" }));
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

it("reuses an opaque LiteLLM request tag without requiring a metadata key", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  proxy.post.mockResolvedValue({
    data: [{ tag: "skill:diagram" }, { tag: "User-Agent: codex" }],
  });
  const onUrlUpdate = vi.fn();
  renderWithProviders(
    <TraceTagFilters
        accessToken=""
      range={{ startMs: 1000, endMs: 2000 }}
    />,
    {
      searchParams: "?agent=code-agent",
      onUrlUpdate,
    },
  );
  await user.click(screen.getByRole("button", { name: "Tags" }));
  await user.click(
    await screen.findByRole("button", { name: "skill:diagram", exact: true }),
  );
  expect(screen.getByText("skill:diagram", { selector: "span" })).toBeVisible();
  await user.keyboard("{Escape}");
  await user.click(screen.getByRole("button", { name: "Investigate tags" }));
  await waitFor(() =>
    expect(onUrlUpdate.mock.lastCall?.[0].queryString).toContain("dialog=new"),
  );
  const params = new URLSearchParams(
    onUrlUpdate.mock.lastCall?.[0].queryString,
  );
  expect(JSON.parse(params.get("tags") ?? "null")).toEqual([
    { key: "tag", value: "skill:diagram" },
  ]);
  expect(params.get("agent")).toBe("code-agent");
});
