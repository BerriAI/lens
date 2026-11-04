import { fireEvent, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it } from "vitest";
import {
  renderWithLens,
  stubGateway,
} from "../../../../../../tests/lens-test-utils";
import { testQueryClient } from "../../../../../../tests/test-utils";
import { TraceTags } from "./TraceTags";
import { runs } from "../../list/runSearch/__fixtures__/runs";
import type { TraceTag } from "../../tags";

afterEach(() => testQueryClient.clear());

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
  await user.click(await screen.findByText("Recorded tags (1)"));
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
  await user.click(screen.getByText("Recorded tags (1)"));
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
