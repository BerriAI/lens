import { describe, expect, it, vi } from "vitest";
import { createLensDemo } from "./createLensDemo";
import { demoDatasetId } from "./demoOptimizations";
import { DEMO_STORAGE_PREFIX, type DemoStorage } from "./storage";
import {
  comparisonRows,
  defaultConfig,
  exampleJudgment,
  modelEligibility,
  presetModels,
  recommendedModel,
  type OptimizationConfig,
} from "../../optimizations/model";

function memoryStorage(): DemoStorage {
  const values = new Map<string, string>();
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => {
      values.set(key, value);
    },
    removeItem: (key) => {
      values.delete(key);
    },
  };
}
const offline = (): OptimizationConfig => ({
  ...defaultConfig("offline"),
  datasetId: demoDatasetId("support_agent"),
  datasetName: "Customer support",
});

describe("model optimization demo", () => {
  it("starts with a completed result per mode and an offline dataset of linked cases", async () => {
    const demo = createLensDemo();
    const seeds = demo.optimizations!.snapshot().runs;
    expect(seeds.map((run) => [run.config.mode, run.config.judge, run.status])).toEqual([
      ["live", "feedback", "completed"],
      ["live", "llm", "completed"],
      ["shadow", "llm", "completed"],
      ["offline", "llm", "completed"],
    ]);
    expect(new Set(seeds.map((run) => run.config.agent)).size).toBe(seeds.length);
    const result = demo.optimizations!.snapshot().runs.find((run) => run.config.mode === "offline")!;
    const dataset = await demo.lens.datasets.get(result.config.datasetId);
    expect(dataset.cases.length).toBeGreaterThan(30);
    expect(result.target).toBe(dataset.cases.length);
    expect((await demo.traces.trace(dataset.cases[0].source.trace_id)).summary.agent_names).toContain(
      dataset.agent_name,
    );
  });
  it.each(["Frontier", "Balanced", "Flash", "Auto"] as const)("limits %s to models the agent can access", (tier) => {
    const selected = presetModels("release_agent", tier);
    expect(selected.length).toBeGreaterThan(0);
    expect(selected.map((id) => modelEligibility("release_agent", id))).toEqual(selected.map(() => null));
    expect(selected).not.toContain("claude-opus-5-5");
  });
  it("rejects an unavailable model even if a caller bypasses the picker", async () => {
    await expect(
      createLensDemo().optimizations!.create({
        ...defaultConfig("live"),
        models: ["muse-wonder-1.0"],
      }),
    ).rejects.toThrow("available candidate");
  });
  it("requires a full-agent connection, then retains it and the comparison after reload", async () => {
    const storage = memoryStorage();
    const demo = createLensDemo(Date.now(), storage);
    const config = { ...offline(), scope: "agent" as const };
    await expect(demo.optimizations!.create(config)).rejects.toThrow("Connect this agent");
    demo.optimizations!.connect(config.agent);
    const id = await demo.optimizations!.create(config);
    const reloaded = createLensDemo(Date.now(), storage).optimizations!.snapshot();
    expect(reloaded.connectedAgents).toContain(config.agent);
    expect(reloaded.runs.find((run) => run.id === id)?.config.scope).toBe("agent");
  });
  it("replays a pinned dataset revision and retains the exact inputs after later edits", async () => {
    const storage = memoryStorage();
    const demo = createLensDemo(Date.now(), storage);
    const original = await demo.lens.datasets.get(offline().datasetId);
    const id = await demo.optimizations!.create(offline());
    const snapshot = demo.optimizations!.snapshot().runs.find((run) => run.id === id)!.examples;
    const saved = await demo.lens.datasets.saveRevision(original.id, {
      base_revision: original.revision,
      cases: original.cases.map((item, index) => ({
        ...item,
        included: index !== 0,
        reply: "Changed after comparison",
      })),
    });
    expect(demo.optimizations!.snapshot().runs.find((run) => run.id === id)!.examples).toEqual(snapshot);
    const reloaded = createLensDemo(Date.now(), storage);
    expect((await reloaded.lens.datasets.get(original.id)).revision).toBe(saved.revision);
    expect((await reloaded.lens.datasets.get(original.id, original.revision)).cases[0].reply).toBe(
      original.cases[0].reply,
    );
    const next = await reloaded.optimizations!.create({
      ...offline(),
      revision: saved.revision,
    });
    expect(reloaded.optimizations!.snapshot().runs.find((run) => run.id === next)!.target).toBe(
      original.cases.length - 1,
    );
  });
  it("pauses progress, resumes it, and completes a finite offline batch", async () => {
    const store = createLensDemo().optimizations!;
    const id = await store.create(offline());
    store.setStatus(id, "paused");
    store.advance();
    expect(store.snapshot().runs.find((run) => run.id === id)?.completed).toBe(0);
    store.setStatus(id, "running");
    store.advance();
    expect(store.snapshot().runs.find((run) => run.id === id)!.completed).toBeGreaterThan(0);
    Array.from({ length: 8 }).forEach(() => store.advance());
    const run = store.snapshot().runs.find((run) => run.id === id)!;
    expect(run.status).toBe("completed");
    expect(run.completed).toBe(run.target);
  });
  it("scores shadow candidates on the same sampled calls and avoids recommending a quality regression", () => {
    const run = createLensDemo()
      .optimizations!.snapshot()
      .runs.find((run) => run.config.mode === "shadow")!;
    const rows = comparisonRows(run);
    expect(rows.map((row) => row.count)).toEqual(rows.map(() => run.completed));
    const best = recommendedModel(run)!;
    const cheapest = rows.toSorted((a, b) => a.cost - b.cost)[0];
    expect(cheapest.qualifies).toBe(false);
    expect(best.id).not.toBe(cheapest.id);
    expect(best.qualifies).toBe(true);
    expect(recommendedModel({ ...run, completed: 2 })).toBeUndefined();
    expect(
      recommendedModel({
        ...run,
        config: { ...run.config, mode: "live", traffic: 100 },
      }),
    ).toBeUndefined();
  });
  it("never sends requests while creating, connecting, scoring, or reloading", async () => {
    const network = vi.spyOn(globalThis, "fetch");
    const demo = createLensDemo(Date.now(), memoryStorage());
    demo.optimizations!.connect("support_agent");
    await demo.optimizations!.create({ ...offline(), scope: "agent" });
    demo.optimizations!.advance();
    await demo.lens.evalRuns.runDemo!("support-quality");
    expect(network).not.toHaveBeenCalled();
    network.mockRestore();
  });
  it("recovers from invalid persisted demo data", () => {
    const storage = memoryStorage();
    storage.setItem(`${DEMO_STORAGE_PREFIX}optimizations`, "{invalid");
    expect(createLensDemo(Date.now(), storage).optimizations!.snapshot().runs).toHaveLength(4);
  });
  it("shows mostly successful source cases and mixed case judgments for a cheaper model", async () => {
    const demo = createLensDemo();
    const run = demo.optimizations!.snapshot().runs[0];
    const sources = await Promise.all(run.examples.map((example) => demo.traces.trace(example.traceId)));
    expect(sources[0].summary.status).toBe("ok");
    expect(sources.filter((trace) => trace.summary.status === "ok").length / sources.length).toBeGreaterThan(0.8);
    const judgments = run.examples.map((example) => exampleJudgment(example, "gpt-6-luna"));
    expect(judgments.some((result) => result.regression)).toBe(true);
    expect(judgments.filter((result) => !result.regression).length).toBeGreaterThan(
      judgments.filter((result) => result.regression).length,
    );
    const dataset = await demo.lens.datasets.get(run.config.datasetId);
    expect(dataset.cases[0].messages.some((message) => message.role === "tool")).toBe(true);
  });
});
