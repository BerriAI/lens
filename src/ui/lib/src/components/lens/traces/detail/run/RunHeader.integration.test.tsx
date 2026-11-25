import { fireEvent, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { renderWithProviders } from "../../../../../../tests/test-utils";
import { LensHostProvider } from "../../../../../host/LensHost";
import { Tabs } from "../../../../ui/tabs";
import researchTrace from "../../__fixtures__/research_trace.json";
import type { Trace } from "../../types";
import { RunHeader } from "./RunHeader";

it("copies a hosted run link through the dashboard without credentials or stale filters", async () => {
  const previousUrl = window.location.href;
  const origin = window.location.origin;
  const research = researchTrace as Trace;
  const trace = {
    ...research,
    summary: { ...research.summary, trace_ref: "run/ref +1" },
  };
  vi.stubGlobal("parent", {
    location: new URL(
      `${origin}/gateway/ui/lens/?tab=home&token=parent-secret`,
    ),
  });
  window.history.replaceState(
    null,
    "",
    "/gateway/lens/ui/?demo=true&span=s1&q=refund&token=child-secret",
  );
  try {
    const user = userEvent.setup();
    renderWithProviders(
      <LensHostProvider host={{ gatewayFrameNavigation: true }}>
        <Tabs value="steps">
          <RunHeader
            trace={trace}
            handoff={{ text: "Read trace", copied: "Copied" }}
            onBack={() => {}}
            embedded
            refreshing={false}
            onRefresh={() => {}}
            live={false}
            canLive={false}
            onLiveChange={() => {}}
          />
        </Tabs>
      </LensHostProvider>,
    );
    fireEvent.click(screen.getByRole("button", { name: "More run actions" }));
    await user.click(
      await screen.findByRole("menuitem", { name: /copy link/i }),
    );

    expect(await navigator.clipboard.readText()).toBe(
      `${origin}/gateway/ui/lens/?demo=true&trace=${trace.summary.trace_id}&trace_ref=run%2Fref+%2B1`,
    );
  } finally {
    vi.unstubAllGlobals();
    window.history.replaceState(null, "", previousUrl);
  }
});
