import { describe, expect, it } from "vitest";
import {
  agentOrigin,
  connectionProfile,
  namedRunCommand,
} from "./httpConnection";

describe("local API connection setup", () => {
  it("generates a bearer profile containing variable names only", () => {
    expect(connectionProfile("agent-service", "bearer")).toBe(
      '[tool.lens.connections."agent-service"]\nbase_url_env = "AGENT_BASE_URL"\nauth = "bearer"\ntoken_env = "AGENT_API_KEY"\n',
    );
  });
  it("omits credential fields for unauthenticated agents", () => {
    expect(connectionProfile("agent", "none")).toBe(
      '[tool.lens.connections."agent"]\nbase_url_env = "AGENT_BASE_URL"\nauth = "none"\n',
    );
  });
  it.each([
    "https://user:secret@agent.test",
    "https://agent.test/path",
    "https://agent.test?key=secret",
    "https://agent.test/#fragment",
    "file:///tmp/agent",
  ])("rejects %s as an origin", (value) => {
    expect(agentOrigin(value).ok).toBe(false);
  });
  it("accepts local agents and normalizes the origin", () => {
    expect(agentOrigin("http://127.0.0.1:9000/")).toEqual({
      ok: true,
      value: "http://127.0.0.1:9000",
    });
    expect(namedRunCommand("agent-regressions", "http://127.0.0.1:9000")).toBe(
      "export AGENT_BASE_URL='http://127.0.0.1:9000'\nuv run lens eval --name 'agent-regressions'",
    );
  });
  it("quotes shell arguments without evaluating substitutions", () => {
    expect(namedRunCommand("agent'$(cmd)")).toBe(
      "uv run lens eval --name 'agent'\\''$(cmd)'",
    );
  });
});
