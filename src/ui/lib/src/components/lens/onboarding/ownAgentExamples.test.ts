import { describe, expect, it } from "vitest";
import { ownAgentEnvironment } from "./ownAgentExamples";

describe("own-agent environment", () => {
  it("uses the standalone key only for trace uploads and quotes its value", () => {
    const env = ownAgentEnvironment("standalone", "https://lens.example///", "x'y");
    expect(env.split("\n")).toEqual([
      "export LITELLM_TRACING_KEY='x'\\''y'",
      "export OTEL_EXPORTER_OTLP_TRACES_ENDPOINT='https://lens.example/v1/traces'",
      'export OTEL_EXPORTER_OTLP_TRACES_HEADERS="Authorization=Bearer%20$LITELLM_TRACING_KEY"',
      "export OPENAI_API_KEY='<your OpenAI provider key>'",
      "export OPENAI_MODEL='<your OpenAI model>'",
    ]);
  });

  it("uses the supplied gateway mount and never copies a Lens key into gateway setup", () => {
    const env = ownAgentEnvironment("gateway", "https://gateway.example/proxy/", "lens-secret");
    expect(env.split("\n")).toEqual([
      "export LITELLM_API_KEY='<your LiteLLM virtual key>'",
      "export OTEL_EXPORTER_OTLP_TRACES_ENDPOINT='https://gateway.example/proxy/v1/traces'",
      'export OTEL_EXPORTER_OTLP_TRACES_HEADERS="Authorization=Bearer%20$LITELLM_API_KEY"',
      "export OPENAI_API_KEY='<your OpenAI provider key>'",
      "export OPENAI_MODEL='<your OpenAI model>'",
    ]);
  });
});
