import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it } from "vitest";
import { renderWithLens, stubGateway } from "../../../../tests/lens-test-utils";
import { testQueryClient } from "../../../../tests/test-utils";
import { LensHostProvider } from "../../../host/LensHost";
import { liveTracesApi, TracesApiContext } from "../traces/api";
import { LensSettings } from "./LensSettings";

beforeEach(() => testQueryClient.clear());

it.each(["standalone", "embedded"] as const)(
  "opens tracing key creation from %s settings",
  async (surface) => {
    const user = userEvent.setup();
    const gateway = stubGateway();
    const list = { lenses: [], workers: [], tracing_enabled: true };
    gateway.get.mockImplementation((path) => {
      if (path === "/lens/tracing/keys") return [];
      if (path === "/lens") return list;
      if (path === "/model_group/info") return { data: [] };
      return null;
    });
    renderWithLens(
      <LensHostProvider host={{ surface }}>
        <TracesApiContext.Provider value={liveTracesApi("test")}>
          <LensSettings list={list} />
        </TracesApiContext.Provider>
      </LensHostProvider>,
    );
    await user.click(
      screen.getByRole("button", { name: "Manage tracing keys" }),
    );
    expect(
      await screen.findByRole("dialog", { name: "Manage tracing keys" }),
    ).toBeVisible();
    expect(screen.getByRole("textbox", { name: "User ID" })).toBeVisible();
  },
);
