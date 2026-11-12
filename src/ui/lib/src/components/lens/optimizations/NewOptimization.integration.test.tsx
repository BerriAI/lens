import { fireEvent, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { chooseSelectOption, renderWithProviders, testQueryClient } from "../../../../tests/test-utils";
import { LensServicesProvider } from "../data/LensServices";
import { createLensDemo } from "../data/demo/createLensDemo";
import { NewOptimization } from "./NewOptimization";

beforeEach(() => testQueryClient.clear());

it("starts a live comparison with the selected model tier, traffic and judge sampling", async () => {
  const services = createLensDemo();
  const store = services.optimizations!;
  const created = vi.fn();
  const user = userEvent.setup();
  renderWithProviders(
    <LensServicesProvider services={services}>
      <NewOptimization mode="live" store={store} onClose={() => {}} onCreated={created} />
    </LensServicesProvider>,
  );
  fireEvent.change(screen.getByRole("spinbutton", { name: "Traffic percent" }), { target: { value: "15" } });
  await user.click(screen.getByRole("button", { name: "Balanced", exact: true }));
  await chooseSelectOption(user, screen.getByRole("combobox", { name: "Quality measurement" }), "LLM judge");
  await user.click(screen.getByText("Advanced", { exact: true }));
  fireEvent.change(screen.getByRole("spinbutton", { name: "Judge baseline percent" }), { target: { value: "35" } });
  await user.click(screen.getByRole("button", { name: "Start live test" }));
  await waitFor(() => expect(created).toHaveBeenCalledOnce());
  const run = store.snapshot().runs.find((run) => run.id === created.mock.calls[0][0])!;
  expect(run.config).toMatchObject({
    mode: "live",
    traffic: 15,
    judge: "llm",
    baselineSampling: 35,
    candidateSampling: 100,
  });
  expect(run.config.models).toContain("gpt-6.1-sol");
  expect(run.config.models).not.toContain("muse-spark-1.3");
  expect(run.status).toBe("running");
});

it("connects a full agent and creates an offline run using the selected dataset", async () => {
  const services = createLensDemo();
  const created = vi.fn();
  const user = userEvent.setup();
  renderWithProviders(
    <LensServicesProvider services={services}>
      <NewOptimization mode="offline" store={services.optimizations!} onClose={() => {}} onCreated={created} />
    </LensServicesProvider>,
  );
  await waitFor(() => expect(screen.getByRole("combobox", { name: "Dataset" })).toHaveTextContent("Customer support"));
  await chooseSelectOption(user, screen.getByRole("combobox", { name: "Replay scope" }), "Full agent");
  expect(screen.getByRole("button", { name: "Run comparison" })).toBeDisabled();
  await user.click(screen.getByRole("button", { name: "Connect agent" }));
  await user.click(screen.getByRole("button", { name: "Connect runner" }));
  expect(screen.getByText(/support_agent connected/)).toBeVisible();
  await user.click(screen.getByRole("button", { name: "Run comparison" }));
  await waitFor(() => expect(created).toHaveBeenCalledOnce());
  const run = services.optimizations!.snapshot().runs.find((run) => run.id === created.mock.calls[0][0])!;
  expect(run.config).toMatchObject({
    mode: "offline",
    scope: "agent",
    datasetId: "demo-support_agent-dataset",
    revision: 1,
  });
  expect(run.target).toBe(
    (await services.lens.datasets.get(run.config.datasetId)).cases.filter((item) => item.included).length,
  );
});
