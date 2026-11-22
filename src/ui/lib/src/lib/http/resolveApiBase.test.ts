import { describe, expect, it } from "vitest";
import { hostedApiPath, hostedApiRoot } from "./resolveApiBase";

describe("hostedApiRoot", () => {
  it.each([
    ["/lens/ui/", "/lens"],
    ["/lens/ui", "/lens"],
    ["/team-a/lens/ui/", "/team-a/lens"],
    ["/lens/ui/index.html", "/lens"],
  ])("finds the gateway address the page %s is served under", (page, root) => {
    expect(hostedApiRoot(page)).toBe(root);
  });

  it.each(["/ui/", "/ui", "/ui/index.html", "/", "/lens/other"])(
    "is null when Lens serves %s itself",
    (page) => {
      expect(hostedApiRoot(page)).toBeNull();
    },
  );
});

describe("hostedApiPath", () => {
  it.each([
    ["/lens/agents", "/team-a/lens/agents"],
    ["/lens", "/team-a/lens"],
    ["/lens?limit=1", "/team-a/lens?limit=1"],
    ["/v1/traces/abc/spans/def", "/team-a/lens/v1/traces/abc/spans/def"],
    ["/auth/session", "/team-a/lens/auth/session"],
    ["/lensing", "/team-a/lens/lensing"],
  ])("sends %s to the gateway's Lens address", (path, hosted) => {
    expect(hostedApiPath("/team-a/lens", path)).toBe(hosted);
  });
});
