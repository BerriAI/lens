import { describe, expect, it } from "vitest";
import {
  evalModule,
  evalSnippet,
  githubAppInstallUrl,
  pyprojectSnippet,
  workflowSnippet,
} from "./connectSnippets";

const target = {
  agent: "Refund Agent",
  dataset: "refunds",
  revision: 7,
  baseUrl: "https://lens.test",
};

describe("connect snippets", () => {
  it("points the workflow at this Lens and keeps credentials in repo secrets", () => {
    const workflow = workflowSnippet(target);
    expect(workflow).toContain("base-url: https://lens.test");
    expect(workflow).toContain("api-key: ${{ secrets.LENS_API_KEY }}");
    expect(workflow).toContain("checks: write");
  });

  it("names the project and pins the dataset revision the eval replays", () => {
    expect(pyprojectSnippet(target)).toContain('project = "Refund Agent"');
    expect(evalSnippet(target)).toContain('data="refunds@7"');
  });

  it.each([
    ["Refund Agent", "refund_agent"],
    ["--", "agent"],
  ])("derives a python module name from %s", (agent, module) => {
    expect(evalModule(agent)).toBe(module);
  });

  it.each([
    [undefined, null],
    ["  ", null],
    ["litellm-lens", "https://github.com/apps/litellm-lens/installations/new"],
  ])("builds the GitHub App install link from %s", (slug, url) => {
    expect(githubAppInstallUrl(slug)).toBe(url);
  });
});
