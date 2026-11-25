import { describe, expect, it } from "vitest";
import { gatewayFrameShareLocation } from "./gatewayFrame";

describe("gatewayFrameShareLocation", () => {
  it.each(["", "/gateway"])(
    "uses the dashboard pathname under prefix '%s' and keeps the frame query",
    (prefix) => {
      const location = new URL(
        `https://gateway.test${prefix}/lens/ui/?demo=true&span=s1`,
      );
      const parent = {
        location: new URL(`https://gateway.test${prefix}/ui/lens/?tab=home`),
      };

      expect(gatewayFrameShareLocation(location, parent)).toEqual({
        origin: location.origin,
        pathname: `${prefix}/ui/lens/`,
        search: "?demo=true&span=s1",
      });
    },
  );

  it("keeps the standalone location even when framed by a same-origin page", () => {
    const location = new URL("https://lens.test/ui/?demo=true");
    const parent = { location: new URL("https://lens.test/other/") };

    expect(gatewayFrameShareLocation(location, parent)).toBe(location);
  });

  it("does not select a parent on another origin", () => {
    const location = new URL("https://gateway.test/lens/ui/?demo=true");
    const parent = { location: new URL("https://other.test/ui/lens/") };

    expect(gatewayFrameShareLocation(location, parent)).toBe(location);
  });

  it("keeps the frame location when the browser denies access to the parent", () => {
    const location = new URL("https://gateway.test/lens/ui/");
    const parent = {
      get location(): Location {
        throw new DOMException("Blocked parent location", "SecurityError");
      },
    };

    expect(gatewayFrameShareLocation(location, parent)).toBe(location);
  });
});
