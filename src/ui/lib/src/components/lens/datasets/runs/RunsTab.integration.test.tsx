import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  renderWithProviders,
  testQueryClient,
} from "../../../../../tests/test-utils";
import {
  renderWithLens,
  stubGateway,
  type GatewayRequest,
} from "../../../../../tests/lens-test-utils";
import { LensServicesProvider } from "../../data/LensServices";
import { createLensDemo } from "../../data/demo/createLensDemo";
import { DatasetsView } from "../DatasetsView";
import type { Dataset, DatasetSummary } from "../types";
import { RunsTab } from "./RunsTab";
import { diff, evalRun, runCase, step, summary, trial } from "./testRuns";
import type { RunCase } from "./types";

vi.mock("../../../../lib/http/requests", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../../lib/http/requests")>()),
  proxyBaseUrl: "",
  getProxyBaseUrl: () => "",
}));

const dataset: Dataset = {
  id: "ds-1",
  name: "Refund agent regressions",
  agent_name: "refund-agent",
  team_id: "",
  created_at: "2026-10-01T10:00:00Z",
  revision: 2,
  created_by: "admin",
  cases: [],
};

const datasetSummary: DatasetSummary = {
  id: dataset.id,
  name: dataset.name,
  agent_name: dataset.agent_name,
  revision: dataset.revision,
  case_count: 0,
  updated_at: "2026-10-02T10:00:00Z",
};

const regression = diff("case-refund", {
  title: "Can I get a refund for order 42?",
  critical: true,
});

const mainRun = evalRun("run-main", {
  version: "1111111aaaa",
  summary: summary({
    passed: 4,
    baseline_run_id: null,
    baseline_version: null,
  }),
});

const redRun = evalRun("run-red", {
  branch: "drop-policy-check",
  version: "2222222bbbb",
  pr: 7,
  summary: summary({
    regressions: [regression],
    gate: {
      passed: false,
      reasons: ["1 critical case regressed against main"],
    },
  }),
});

const erroredRun = evalRun("run-error", {
  branch: "flaky-ci",
  version: "3333333cccc",
  pr: 8,
  status: "failed",
  summary: null,
  failure: "dataset revision 9 not found",
});

const failedCheck = { scorer: "called_before", passed: false };
const cases: Record<string, RunCase> = {
  "run-main": runCase(true, [
    trial(
      1,
      [
        step("lookup_order", 0),
        step("check_refund_policy", 10_000_000),
        step("issue_refund", 20_000_000),
      ],
      {
        checks: [{ ...failedCheck, passed: true }],
      },
    ),
  ]),
  "run-red": runCase(false, [
    trial(1, [step("lookup_order", 0), step("issue_refund", 20_000_000)], {
      checks: [failedCheck],
    }),
  ]),
};

let proxy = stubGateway();

const serve = (path: string, request: GatewayRequest) => {
  if (path === "/lens/datasets") return [datasetSummary];
  if (path === `/lens/datasets/${dataset.id}`) return dataset;
  if (path === "/lens/evals/runs" && request.query.dataset === dataset.id)
    return [redRun, erroredRun, mainRun];
  const run = [mainRun, redRun, erroredRun].find(
    (item) => path === `/lens/evals/runs/${item.id}`,
  );
  if (run) return run;
  const runId = path.match(
    /^\/lens\/evals\/runs\/([^/]+)\/cases\/case-refund$/,
  )?.[1];
  if (runId && cases[runId]) return cases[runId];
  throw new Error(`unexpected GET ${path} ${JSON.stringify(request.query)}`);
};

beforeEach(() => {
  testQueryClient.clear();
  proxy = stubGateway();
  proxy.get.mockImplementation(serve);
});

const toolNames = (trace: string) =>
  within(
    within(screen.getByRole("region", { name: `${trace} trace` })).getByRole(
      "list",
      { name: "Tool calls" },
    ),
  )
    .getAllByRole("listitem")
    .map((item) => item.getAttribute("aria-label"));

async function expectComparison() {
  await waitFor(() =>
    expect(screen.getByLabelText("Diagnosis")).toHaveTextContent(
      "regressed: never called check_refund_policy, which main called · called_before failed 1/1 trials",
    ),
  );
  expect(
    await screen.findAllByRole("list", { name: "Tool calls" }),
  ).toHaveLength(2);
  expect(toolNames("main")).toEqual([
    "lookup_order",
    "check_refund_policy (missing in candidate)",
    "issue_refund",
  ]);
  expect(toolNames("candidate")).toEqual(["lookup_order", "issue_refund"]);
  expect(
    screen.getByRole("region", { name: "Case comparison" }),
  ).toHaveTextContent(regression.title);
}

describe("Dataset runs", () => {
  it("opens a red run straight onto its first regression and restores it from the URL", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    const first = renderWithLens(<DatasetsView />, {
      searchParams: `?tab=datasets&dataset=${dataset.id}`,
      onUrlUpdate,
    });

    await user.click(await screen.findByRole("tab", { name: "Runs" }));
    const pulls = await screen.findByRole("rowgroup", {
      name: "Pull requests",
    });
    const row = within(pulls).getByRole("row", { name: /drop-policy-check/ });
    expect(
      within(row)
        .getAllByRole("cell")
        .map((cell) => cell.textContent),
    ).toEqual([
      "gate failed",
      "drop-policy-check@2222222#7",
      "agent-regressions",
      "3/4",
      "1",
      "0",
      "$0.05",
      "4/4",
      "run-red",
    ]);

    await user.click(
      within(row).getByRole("button", { name: /drop-policy-check/ }),
    );
    expect(
      await screen.findByRole("list", { name: "Gate reasons" }),
    ).toHaveTextContent("1 critical case regressed against main");
    expect(screen.getByRole("banner", { name: "Run" })).toHaveTextContent(
      "vsmain@1111111",
    );
    expect(
      within(
        screen.getByRole("navigation", { name: "Changed cases" }),
      ).getByRole("button", { current: true }),
    ).toHaveTextContent(regression.title);
    await expectComparison();

    await user.click(
      screen.getByRole("button", { name: /Can I get a refund/ }),
    );
    const url = onUrlUpdate.mock.lastCall?.[0].queryString as string;
    expect(new URLSearchParams(url).get("eval_case")).toBe(regression.case_id);
    first.unmount();
    testQueryClient.clear();

    renderWithLens(<DatasetsView />, { searchParams: url });
    await expectComparison();
    expect(screen.getByRole("tab", { name: "Runs" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("lands on the regressed case from a PR comment link without listing runs", async () => {
    renderWithLens(<DatasetsView />, {
      searchParams: `?tab=datasets&dataset=${dataset.id}&eval_run=${redRun.id}&eval_case=${regression.case_id}`,
    });

    await expectComparison();
    expect(proxy.get).not.toHaveBeenCalledWith(
      "/lens/evals/runs",
      expect.anything(),
    );
  });

  it("shows why an errored run failed instead of saying it is still scoring", async () => {
    const user = userEvent.setup();
    renderWithLens(<DatasetsView />, {
      searchParams: `?tab=datasets&dataset=${dataset.id}&dataset_tab=runs`,
    });

    const row = await screen.findByRole("row", { name: /flaky-ci/ });
    expect(within(row).getByText("gate errored")).toBeInTheDocument();

    await user.click(within(row).getByRole("button", { name: /flaky-ci/ }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "error: dataset revision 9 not found",
    );
    expect(screen.queryByText(/scoring/)).not.toBeInTheDocument();
  });

  it("says so when a PR link names a case that did not change, and clears it on dismiss", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    renderWithLens(<DatasetsView />, {
      searchParams: `?tab=datasets&dataset=${dataset.id}&eval_run=${redRun.id}&eval_case=case-gone`,
      onUrlUpdate,
    });

    const missing = await screen.findByRole("alert");
    expect(missing).toHaveTextContent(
      "Case case-gone did not regress or get fixed in this run.",
    );

    await user.click(within(missing).getByRole("button", { name: "Dismiss" }));
    await expectComparison();
    expect(
      new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString).has(
        "eval_case",
      ),
    ).toBe(false);
  });

  it("drops the run and case when you leave the dataset, so reopening it starts on Cases", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    renderWithLens(<DatasetsView />, {
      searchParams: `?tab=datasets&dataset=${dataset.id}&dataset_tab=runs&eval_run=${redRun.id}&eval_case=${regression.case_id}`,
      onUrlUpdate,
    });
    await expectComparison();

    await user.click(screen.getByRole("button", { name: "Datasets" }));
    await user.click(
      await screen.findByRole("row", { name: new RegExp(dataset.name) }),
    );

    expect(await screen.findByRole("tab", { name: "Cases" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    const params = new URLSearchParams(
      onUrlUpdate.mock.lastCall?.[0].queryString,
    );
    expect(params.get("dataset")).toBe(dataset.id);
    expect(
      ["dataset_tab", "eval_run", "eval_case"].filter((key) => params.has(key)),
    ).toEqual([]);
  });

  it("serves an empty run list in sample mode without calling the proxy", async () => {
    renderWithProviders(
      <LensServicesProvider services={createLensDemo()}>
        <RunsTab datasetId={dataset.id} agentName={dataset.agent_name} />
      </LensServicesProvider>,
    );

    expect(await screen.findByText("No eval runs yet")).toBeInTheDocument();
    expect(proxy.get).not.toHaveBeenCalled();
  });
});
