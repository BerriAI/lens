import { describe, expect, it } from "vitest";
import { workspaceScopeAfterSelect } from "./scope";

const scope = {
  organization_id: "org-a",
  team_id: "team-a",
  project_id: "project-a",
  api_key_hash: "hash-a",
};

describe("workspaceScopeAfterSelect", () => {
  it("clears every descendant when switching organizations", () => {
    expect(
      workspaceScopeAfterSelect(scope, "organization", {
        id: "org-b",
        name: "Organization B",
      }),
    ).toEqual({
      organization_id: "org-b",
      team_id: null,
      project_id: null,
      api_key_hash: null,
    });
  });

  it("retains the organization and clears project and key when showing all teams", () => {
    expect(workspaceScopeAfterSelect(scope, "team", null)).toEqual({
      organization_id: "org-a",
      team_id: null,
      project_id: null,
      api_key_hash: null,
    });
  });

  it("fills parents when a project is selected across teams", () => {
    expect(
      workspaceScopeAfterSelect(scope, "project", {
        id: "project-b",
        name: "Project B",
        organization_id: "org-b",
        team_id: "team-b",
      }),
    ).toEqual({
      organization_id: "org-b",
      team_id: "team-b",
      project_id: "project-b",
      api_key_hash: null,
    });
  });

  it("clears a stale project when the selected key has no project", () => {
    expect(
      workspaceScopeAfterSelect(scope, "key", {
        id: "hash-b",
        name: "Development",
        organization_id: "org-b",
        team_id: "team-b",
        project_id: null,
      }),
    ).toEqual({
      organization_id: "org-b",
      team_id: "team-b",
      project_id: null,
      api_key_hash: "hash-b",
    });
  });

  it("clears only the key when showing all keys in the selected project", () => {
    expect(workspaceScopeAfterSelect(scope, "key", null)).toEqual({
      ...scope,
      api_key_hash: null,
    });
  });
});
