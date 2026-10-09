import { describe, expect, it } from "vitest";
import { setupPrompt, type SetupConnection } from "./setupPrompt";

describe("agent setup prompt", () => {
  it("copies deployment addresses without URL credentials, queries or fragments", () => {
    const prompt = setupPrompt({
      standalone: true,
      goal: "tracing",
      baseUrl: "https://admin:private-password@lens.test/prefix/?token=private-token#private-fragment",
      connection: {
        connected: true,
        status: { storage_ready: true },
        url: "https://traces.test/collector/?key=private-key",
      },
    });
    expect(prompt).toContain('Lens API base: "https://lens.test/prefix"');
    expect(prompt).toContain('Reported tracing base: "https://traces.test/collector"');
    expect(prompt).not.toContain("private-");
    expect(prompt).toContain("a LiteLLM gateway is optional");
  });

  it.each([
    [undefined, "has not been verified"],
    [{ configured: false, connected: false, status: {} }, "Check for an existing Lens service"],
    [{ configured: true, connected: false, status: {} }, "Check reachability and API compatibility"],
    [{ connected: true, status: { storage_ready: false } }, "Diagnose ClickHouse first"],
    [{ connected: true, status: { storage_ready: true } }, "Reuse this installation"],
  ] satisfies readonly (readonly [SetupConnection | undefined, string])[])(
    "preserves the observed service state %j",
    (connection, guidance) => {
      const prompt = setupPrompt({ standalone: false, goal: "start", connection });
      expect(prompt).toContain(guidance);
      expect(prompt).toContain("existing LiteLLM admin dashboard");
      expect(prompt).toContain("Recheck the observed state");
    },
  );

  it.each(["javascript:private-value", "invalid private-value"])("omits unusable address %s", (baseUrl) => {
    const prompt = setupPrompt({ standalone: true, goal: "start", baseUrl });
    expect(prompt).not.toContain("private-value");
    expect(prompt).toContain("Discover the deployment's reachable API address");
  });

  it("carries the selected eval without requiring the user to look it up again", () => {
    const evaluation = { name: "invoice-accuracy", agent: "invoice-agent", dataset: "dataset-id", revision: 3 };
    const prompt = setupPrompt({ standalone: true, goal: "evals", evaluation });
    expect(prompt).toContain(`Selected eval: ${JSON.stringify(evaluation)}`);
    expect(prompt).toContain("whether to add a pull-request check");
  });
});
