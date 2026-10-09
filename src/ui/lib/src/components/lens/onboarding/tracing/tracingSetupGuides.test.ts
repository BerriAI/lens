import { describe, expect, it } from "vitest";
import { FRAMEWORKS, standaloneFrameworkSnippet } from "./tracingSetupGuides";

describe("Standalone agent configuration", () => {
  it.each(FRAMEWORKS)("should safely include the chosen agent name in $label configuration", (guide) => {
    const name = 'Moyai "support"\n${LITELLM_API_KEY} LITELLM_API_KEY';
    const snippet = standaloneFrameworkSnippet(guide, "https://lens.example", name);
    expect(snippet).toContain(JSON.stringify(name));
    expect(snippet).not.toContain("{AGENT_NAME_LITERAL}");
    expect(snippet).not.toContain("research_agent");
    if (guide.quickstart.includes('f"gen_ai.agent.name={AGENT_NAME}"')) {
      expect(snippet).toContain('f"gen_ai.agent.name={AGENT_NAME}"');
    }
  });

  it.each(FRAMEWORKS.filter((guide) => !guide.existingModel))(
    "sends $label model calls to a provider and trace exports to Lens",
    (guide) => {
      const snippet = standaloneFrameworkSnippet(guide, "https://lens.example/observe");
      expect(snippet).not.toContain("LITELLM_API_KEY");
      expect(snippet).not.toContain("https://lens.example/observe/v1/chat");
      expect(snippet).toContain(guide.id === "claude" ? "https://api.anthropic.com" : "https://api.openai.com/v1");
      expect(snippet).toContain(guide.id === "claude" ? "ANTHROPIC_API_KEY" : "OPENAI_API_KEY");
    },
  );

  it.each(FRAMEWORKS.filter((guide) => guide.existingModel))(
    "keeps $label model settings while configuring the dedicated tracing key",
    (guide) => {
      const snippet = standaloneFrameworkSnippet(guide, "https://lens.example/observe");
      expect(snippet).toContain("https://lens.example/observe/v1/traces");
      expect(snippet).toContain("LITELLM_TRACING_KEY");
      expect(snippet).not.toContain("OPENAI_API_KEY");
      expect(snippet).not.toContain("ANTHROPIC_API_KEY");
    },
  );
});
