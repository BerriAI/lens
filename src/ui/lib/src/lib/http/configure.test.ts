import { afterEach, describe, expect, it, vi } from "vitest";
import { configureLensHttp } from "./configure";
import {
  getAuthHeaderName,
  getAuthToken,
  getRequestBaseUrl,
  reportError,
} from "./runtime";
import { resolveLogoSrc } from "../assetPaths";
import { routeSegmentForPathname, uiHref } from "../../utils/uiHref";

const defaults = { getBaseUrl: () => "", getAuthToken: () => null };

afterEach(() => {
  configureLensHttp(defaults);
  vi.unstubAllEnvs();
});

describe("Lens host HTTP configuration", () => {
  it("resolves the current host prefix for UI links and assets after configuration", () => {
    vi.stubEnv("NODE_ENV", "production");
    const root = vi.fn(() => "/gateway");
    configureLensHttp({ ...defaults, getServerRootPath: root });
    expect(uiHref("lens")).toBe("/gateway/ui/lens");
    expect(resolveLogoSrc("/ui/logo.svg")).toBe("/gateway/ui/logo.svg");

    root.mockReturnValue("/nested/gateway");
    expect(uiHref("lens")).toBe("/nested/gateway/ui/lens");
    expect(routeSegmentForPathname("/nested/gateway/ui/lens/")).toBe("lens");
    expect(resolveLogoSrc("/ui/logo.svg")).toBe("/nested/gateway/ui/logo.svg");

    root.mockReturnValue("/");
    expect(uiHref("lens")).toBe("/ui/lens");
    expect(resolveLogoSrc("/ui/logo.svg")).toBe("/ui/logo.svg");
  });

  it("supports static prefixes and resets an earlier host getter when reconfigured", () => {
    vi.stubEnv("NODE_ENV", "production");
    configureLensHttp({ ...defaults, getServerRootPath: () => "/old" });
    configureLensHttp({ ...defaults, serverRootPath: "/static" });
    expect(uiHref("lens")).toBe("/static/ui/lens");
    expect(resolveLogoSrc("/ui/logo.svg")).toBe("/static/ui/logo.svg");
    configureLensHttp(defaults);
    expect(uiHref("lens")).toBe("/ui/lens");
  });

  it("retains live session and connection getters through refresh and logout", () => {
    const base = vi.fn(() => "https://gateway.example");
    const token = vi.fn<() => string | null>(() => "initial-token");
    const header = vi.fn(() => "X-Gateway-Key");
    const error = vi.fn();
    configureLensHttp({
      getBaseUrl: base,
      getAuthToken: token,
      getAuthHeaderName: header,
      onError: error,
    });
    expect(getRequestBaseUrl()).toBe("https://gateway.example");
    expect(getAuthToken()).toBe("initial-token");
    expect(getAuthHeaderName()).toBe("X-Gateway-Key");
    token.mockReturnValue("refreshed-token");
    base.mockReturnValue("https://worker.example");
    header.mockReturnValue("Authorization");
    expect(getAuthToken()).toBe("refreshed-token");
    expect(getRequestBaseUrl()).toBe("https://worker.example");
    expect(getAuthHeaderName()).toBe("Authorization");
    token.mockReturnValue(null);
    expect(getAuthToken()).toBeNull();
    reportError("Session expired");
    expect(error).toHaveBeenCalledWith("Session expired");
  });
});
