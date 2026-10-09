import { act, renderHook, waitFor } from "@testing-library/react";
import { NuqsTestingAdapter } from "nuqs/adapters/testing";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useLensRoute } from "../route";
import { useAgentSelection } from "./useAgentSelection";

beforeEach(() => window.localStorage.clear());

describe("agent navigation", () => {
  it("should keep the agents directory open without inserting a remembered agent into its URL", () => {
    window.localStorage.setItem("litellm.lens.agent.demo", "research_agent");
    const onUrlUpdate = vi.fn();
    const wrapper = ({ children }: PropsWithChildren) => (
      <NuqsTestingAdapter searchParams="?tab=agents&demo=true" onUrlUpdate={onUrlUpdate} hasMemory>
        {children}
      </NuqsTestingAdapter>
    );
    const { result } = renderHook(() => useAgentSelection(true, ["support_agent", "research_agent"]), { wrapper });

    expect(result.current.agent).toBe("research_agent");
    expect(onUrlUpdate).not.toHaveBeenCalled();
  });

  it("should open the chosen agent and clear stale details and filters in one history entry", async () => {
    const onUrlUpdate = vi.fn();
    const searchParams = new URLSearchParams({
      tab: "agents",
      demo: "true",
      agent: "support_agent",
      agent_search: "research",
      trace: "old-trace",
      trace_ref: "old-ref",
      span: "old-span",
      fullscreen: "true",
      view: "thread",
      span_tab: "request",
      steps_q: "old-step",
      errors: "true",
      lens: "old-investigation",
      issue: "old-issue",
      run: "old-run",
      section: "checks",
      finding: "old-finding",
      finding_status: "closed",
      kind: "pattern",
      evidence: "old-evidence",
      evidence_span: "old-evidence-span",
      dialog: "edit",
      target: "old-target",
      dataset: "old-dataset",
      revision: "2",
      case: "old-case",
      dataset_tab: "runs",
      eval: "old-eval",
      eval_run: "old-eval-run",
      eval_case: "old-eval-case",
      q: "agent:support_agent",
      status: "error",
      search: "old-search",
      inbox_agent: "support_agent",
      priority: "high",
      setup: "lens",
      hours: "48",
      from: "100",
      to: "200",
      unrelated: "kept",
    });
    const wrapper = ({ children }: PropsWithChildren) => (
      <NuqsTestingAdapter searchParams={searchParams} onUrlUpdate={onUrlUpdate} hasMemory>
        {children}
      </NuqsTestingAdapter>
    );
    const { result } = renderHook(() => useAgentSelection(true, ["support_agent", "research_agent"]), { wrapper });

    act(() => result.current.select("research_agent"));

    await waitFor(() => expect(onUrlUpdate).toHaveBeenCalledOnce());
    expect(Object.fromEntries(onUrlUpdate.mock.lastCall![0].searchParams)).toEqual({
      tab: "traces",
      demo: "true",
      agent: "research_agent",
      agent_search: "research",
      hours: "48",
      from: "100",
      to: "200",
      unrelated: "kept",
    });
    expect(onUrlUpdate.mock.lastCall![0].options.history).toBe("push");
    expect(window.localStorage.getItem("litellm.lens.agent.demo")).toBe("research_agent");
    expect(window.localStorage.getItem("litellm.lens.agent")).toBeNull();
  });

  it("should clear the directory search and agent when leaving the demo session", async () => {
    const onUrlUpdate = vi.fn();
    const wrapper = ({ children }: PropsWithChildren) => (
      <NuqsTestingAdapter
        searchParams="?tab=agents&demo=true&agent=research_agent&agent_search=research"
        onUrlUpdate={onUrlUpdate}
        hasMemory
      >
        {children}
      </NuqsTestingAdapter>
    );
    const { result } = renderHook(useLensRoute, { wrapper });

    act(() => result.current.setDemo(false));

    await waitFor(() => expect(onUrlUpdate).toHaveBeenCalledOnce());
    expect(Object.fromEntries(onUrlUpdate.mock.lastCall![0].searchParams)).toEqual({ tab: "agents" });
  });

  it.each([true, false])("should clear eval selection when changing demo to %s", async (demo) => {
    const onUrlUpdate = vi.fn();
    const wrapper = ({ children }: PropsWithChildren) => (
      <NuqsTestingAdapter
        searchParams={`?tab=evals&demo=${!demo}&eval=arithmetic&eval_run=old-run&eval_case=old-case&dataset_tab=runs&unrelated=kept`}
        onUrlUpdate={onUrlUpdate}
        hasMemory
      >
        {children}
      </NuqsTestingAdapter>
    );
    const { result } = renderHook(useLensRoute, { wrapper });

    act(() => result.current.setDemo(demo));

    await waitFor(() => expect(onUrlUpdate).toHaveBeenCalledOnce());
    expect(Object.fromEntries(onUrlUpdate.mock.lastCall![0].searchParams)).toEqual({
      tab: "evals",
      ...(demo ? { demo: "true" } : {}),
      unrelated: "kept",
    });
    expect(onUrlUpdate.mock.lastCall![0].options.history).toBe("push");
  });
});
