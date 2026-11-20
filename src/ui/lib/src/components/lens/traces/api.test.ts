import { describe, expect, it, vi } from "vitest";

import { agentHandoffText } from "./api";
import { getProxyBaseUrl } from "../../../lib/http/requests";

vi.mock("../../../lib/http/requests", () => ({ getProxyBaseUrl: vi.fn(() => "https://proxy.test/root/"),
}));

describe("agent handoff", () => {
  it("requests bounded trace pages and explains how to retrieve captured content", () => {
    const text = agentHandoffText("trace-one", null, "source-one");
    expect(text).toContain("'https://proxy.test/root/v1/traces/trace-one?trace_ref=source-one&page_size=200'");
    expect(text).toContain("Follow next_cursor");
    expect(text).toContain("/v1/traces/trace-one/spans/{span_id}");
    expect(text).toContain("using the same trace_ref");
  });

  it("selects one span and keeps reserved characters inside their original parameter", () => {
    const text = agentHandoffText("trace/one", "step?two", "source&cursor=other");
    expect(text).toContain(
      "'https://proxy.test/root/v1/traces/trace%2Fone/spans/step%3Ftwo?trace_ref=source%26cursor%3Dother'",
    );
    expect(text).toContain("this step's captured input, output, and attributes");
  });

  it("quotes the proxy URL as shell data", () => {
    vi.mocked(getProxyBaseUrl).mockReturnValueOnce("https://proxy.test/a'b/$(printf injected)");
    expect(agentHandoffText("trace-one", "span-one")).toContain(
      "'https://proxy.test/a'\"'\"'b/$(printf injected)/v1/traces/trace-one/spans/span-one'",
    );
  });
});

it("keeps team and key constraints in the copied trace command", () => {
  const text = agentHandoffText("trace-one", undefined, "ref-a", {
    team_id: "team&a",
    api_key_hash: "hash-a",
  });
  expect(text).toContain("team_id=team%26a&api_key_hash=hash-a&trace_ref=ref-a&page_size=200");
});
