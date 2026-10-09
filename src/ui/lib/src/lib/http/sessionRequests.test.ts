import { afterEach, describe, expect, it, vi } from "vitest";
import { liveLensServices } from "../../components/lens/data/LensServices";
import { configureLensHttp } from "./configure";
import { createApiClient } from "./client";

afterEach(() => {
  vi.unstubAllGlobals();
  configureLensHttp({ getBaseUrl: () => "", getAuthToken: () => null });
});

describe("Lens session requests", () => {
  it("should bypass cached redirects when checking the browser session", async () => {
    const session = { user_id: "admin", user_role: "proxy_admin" };
    const fetcher = vi.fn<typeof fetch>(async (input, init) => {
      const request = new Request(input, init);
      expect(request.url).toBe("https://lens.test/auth/session");
      expect(request.cache).toBe("no-store");
      expect(request.credentials).toBe("same-origin");
      return Response.json(session);
    });
    const api = createApiClient({
      getBaseUrl: () => "https://lens.test",
      fetchImpl: fetcher,
    });

    await expect(
      api.get("/auth/session", { cache: "no-store" }),
    ).resolves.toEqual(session);
    expect(fetcher).toHaveBeenCalledOnce();
  });

  it.each(["", "lens-credential"])(
    "preserves the selected authentication mode for %j",
    async (token) => {
      configureLensHttp({
        getBaseUrl: () => "https://lens.test/mounted",
        getAuthToken: () => null,
        getAuthHeaderName: () => "x-lens-authorization",
      });
      const fetcher = vi.fn<typeof fetch>(async (input, init) => {
        const request =
          input instanceof Request ? input : new Request(input, init);
        expect(request.url).toMatch(
          /^https:\/\/lens.test\/mounted\/(lens|v1\/traces)/,
        );
        expect(request.headers.get("x-lens-authorization")).toBe(
          token ? `Bearer ${token}` : null,
        );
        expect(request.headers.get("Authorization")).toBeNull();
        expect(request.credentials).toBe("same-origin");
        return Response.json([]);
      });
      vi.stubGlobal("fetch", fetcher);
      const services = liveLensServices(token);
      await services.lens.lenses();
      await services.lens.datasets.list();
      await services.traces.list({ startMs: 1, endMs: 2 });
      expect(fetcher).toHaveBeenCalledTimes(3);
    },
  );
});
