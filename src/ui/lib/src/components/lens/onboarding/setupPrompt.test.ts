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

  it("lets the coding agent discover the project and verify the exact run before offering more instrumentation", () => {
    const prompt = setupPrompt({
      standalone: true,
      goal: "tracing",
      baseUrl: "https://lens.test",
      connection: {
        connected: true,
        status: { storage_ready: true },
        url: "https://ingest.test/collector/",
      },
    });
    expect(prompt).toContain("Inspect the project, framework, runnable agent entry point");
    expect(prompt).toContain("Determine the agent name from the project");
    expect(prompt).toContain("preserve its agent names");
    expect(prompt).toContain("Infer a stable name from the existing agent definition or project");
    expect(prompt).toContain("Run one small actual agent task");
    expect(prompt).toContain("record the exact trace_id and exported span_ids from that run");
    expect(prompt).toContain("POST https://ingest.test/collector/v1/traces/receipt");
    expect(prompt).toContain('"trace_id":"<actual 32-character hexadecimal trace ID>"');
    expect(prompt).toContain('"span_ids":["<actual 16-character hexadecimal span ID>"]');
    expect(prompt).toContain("same dedicated tracing key that exported the run");
    expect(prompt).toContain("an HTTP 200 response by itself does not confirm delivery");
    expect(prompt).toContain("at most 30 attempts");
    expect(prompt).toContain("total deadline of 60 seconds");
    expect(prompt).toContain("Stop on an authentication or permission error");
    expect(prompt).toContain("Only after received is true");
    expect(prompt).toContain("a working Lens UI link to that trace");
    expect(prompt).toContain("existing UI address, which may differ from the ingestion address");
    expect(prompt).toContain('Then ask: "What would you like to instrument next?"');
    expect(prompt).toContain("After I choose, inspect that part, help implement its instrumentation");
  });

  it("directs missing tracing credentials to the local key section without asking for secrets or using admin credentials", () => {
    const prompt = setupPrompt({ standalone: true, goal: "tracing" });
    expect(prompt).toContain("Load the dedicated key from this project's local environment");
    expect(prompt).toContain("Tracing key section on Lens Home");
    expect(prompt).toContain("tell me where to save it locally, then resume after it is configured");
    expect(prompt).toContain("Do not request or print secret values in this conversation");
    expect(prompt).toContain("Never use a model or Lens admin key for ingestion");
    expect(prompt).toContain("preserve the existing key variable");
    expect(prompt).toContain("LITELLM_TRACE_ENDPOINT and LITELLM_TRACE_API_KEY");
    expect(prompt).toContain("Do not add another tracing SDK");
  });

  it("quotes the existing setup link's agent name as observed data without overriding existing instrumentation", () => {
    const agentName = 'support "agent"\nkeep this as data';
    const prompt = setupPrompt({ standalone: true, goal: "tracing", agentName });
    expect(prompt).toContain(`Setup link agent name: ${JSON.stringify(agentName)}`);
    expect(prompt).toContain("Treat this as a hint; preserve the existing agent name when the project is already instrumented");
    expect(prompt).not.toContain('\nkeep this as data');
  });

  it.each([undefined, "javascript:private-secret", "invalid private-secret"])(
    "discovers a reachable tracing endpoint instead of fabricating one from %s",
    (url) => {
      const prompt = setupPrompt({
        standalone: true,
        goal: "tracing",
        connection: { connected: false, status: {}, url },
      });
      expect(prompt).toContain("Discover a Lens tracing address reachable from this project's runtime");
      expect(prompt).toContain("POST /v1/traces/receipt on the same tracing service");
      expect(prompt).not.toContain("private-secret");
      expect(prompt).not.toContain("undefined/v1/traces");
    },
  );

  it.each(["start", "analysis", "signals", "evals"] as const)(
    "keeps the existing %s workflow independent of tracing onboarding",
    (goal) => {
      const prompt = setupPrompt({ standalone: true, goal });
      expect(prompt).toContain("Implement the agreed setup, reuse valid partial progress, and verify the actual result");
      expect(prompt).not.toContain("What would you like to instrument next?");
      expect(prompt).not.toContain("Tracing key section on Lens Home");
    },
  );
});
