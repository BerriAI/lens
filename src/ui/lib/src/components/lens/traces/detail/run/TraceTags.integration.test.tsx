import { fireEvent, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import {
  renderWithLens,
  stubGateway,
} from "../../../../../../tests/lens-test-utils";
import { testQueryClient } from "../../../../../../tests/test-utils";
import { TraceTags } from "./TraceTags";
import { runs } from "../../list/runSearch/__fixtures__/runs";
import type { TraceTag } from "../../tags";

afterEach(() => testQueryClient.clear());

it("shows upstream LiteLLM tags as read-only filters and preserves their exact value", async () => {
  const user = userEvent.setup();
  const gateway = stubGateway();
  gateway.get.mockResolvedValue({
    recorded: [{ key: "tag", value: "skill:diagram" }],
    manual: [],
    editable: false,
  });
  const onUrlUpdate = vi.fn();
  renderWithLens(<TraceTags summary={runs[0]} accessToken="test" />, {
    onUrlUpdate,
  });
  await user.click(
    await screen.findByRole("button", { name: "skill:diagram", exact: true }),
  );
  await waitFor(() =>
    expect(onUrlUpdate.mock.lastCall?.[0].queryString).toContain("skill"),
  );
  const params = new URLSearchParams(
    onUrlUpdate.mock.lastCall?.[0].queryString,
  );
  expect(JSON.parse(params.get("tags") ?? "null")).toEqual([
    { key: "tag", value: "skill:diagram" },
  ]);
  expect(
    screen.queryByRole("button", { name: "Save tag" }),
  ).not.toBeInTheDocument();
});

it("persists manual tags across remounts and removes them without editing recorded metadata", async () => {
  const user = userEvent.setup();
  const gateway = stubGateway();
  let manual: TraceTag[] = [];
  const response = () => ({
    recorded: [{ key: "department", value: "engineering" }],
    manual,
    editable: true,
  });
  gateway.get.mockImplementation(response);
  gateway.put.mockImplementation((_path, request) => {
    manual = (request.body as { tags: TraceTag[] }).tags;
    return response();
  });
  const first = renderWithLens(
    <TraceTags summary={runs[0]} accessToken="test" />,
  );
  await user.click(await screen.findByText("Recorded metadata (1)"));
  expect(screen.getByText("department: engineering")).toBeVisible();
  fireEvent.change(screen.getByLabelText("Tag key"), {
    target: { value: "skill" },
  });
  fireEvent.change(screen.getByLabelText("Tag value"), {
    target: { value: "diagram" },
  });
  await user.click(screen.getByRole("button", { name: "Save tag" }));
  expect(
    await screen.findByRole("button", { name: "Remove tag skill: diagram" }),
  ).toBeVisible();
  first.unmount();
  testQueryClient.clear();
  renderWithLens(<TraceTags summary={runs[0]} accessToken="test" />);
  await user.click(
    await screen.findByRole("button", { name: "Remove tag skill: diagram" }),
  );
  expect(
    await screen.findByRole("button", { name: "Save tag" }),
  ).toBeDisabled();
  expect(manual).toEqual([]);
  await user.click(screen.getByText("Recorded metadata (1)"));
  expect(screen.getByText("department: engineering")).toBeVisible();
});

it("shows a retryable error when tags cannot be read", async () => {
  const user = userEvent.setup();
  const gateway = stubGateway();
  gateway.get.mockImplementationOnce(() => {
    throw new Error("Storage unavailable");
  });
  gateway.get.mockImplementation(() => ({
    recorded: [],
    manual: [],
    editable: false,
  }));
  renderWithLens(<TraceTags summary={runs[0]} accessToken="test" />);
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Tags are unavailable",
  );
  await user.click(screen.getByRole("button", { name: "Retry tags" }));
  expect(await screen.findByText("No tags yet")).toBeVisible();
});
