export interface ApiBaseInputs {
  /**
   * Absolute API origin to target instead of same-origin. Comes from the
   * NEXT_PUBLIC_BASE_URL build env (dev / split-origin) or the proxy_base_url
   * the backend reports in its UI config. Empty/unset means same-origin.
   */
  explicitBase?: string | null;
  /**
   * The proxy's mount path when it sits under a sub-path (e.g. "/litellm"
   * behind a reverse proxy). "/" or empty means mounted at the root.
   */
  serverRootPath?: string | null;
}

export const normalizeRootPath = (serverRootPath: string | null | undefined): string => {
  const trimmed = (serverRootPath ?? "").trim();
  if (trimmed === "" || trimmed === "/") return "";
  const withLeadingSlash = trimmed.startsWith("/") ? trimmed : `/${trimmed}`;
  return withLeadingSlash.replace(/\/+$/, "");
};

/**
 * Resolve the single API base string that every request is prefixed with.
 *
 * Same-origin is represented as "" so requests stay relative and work behind
 * any domain or reverse proxy without hardcoding an origin. An explicit base is
 * used verbatim (trailing slash trimmed). The server root path is appended
 * unless the base already ends with it.
 */
export const resolveApiBase = ({ explicitBase, serverRootPath }: ApiBaseInputs): string => {
  const base = (explicitBase ?? "").trim().replace(/\/+$/, "");
  const rootPath = normalizeRootPath(serverRootPath);
  if (rootPath === "" || base.endsWith(rootPath)) return base;
  return `${base}${rootPath}`;
};

export interface RequestUrlInputs {
  /** Base registered at runtime (a split-origin proxy or worker URL); empty means none. */
  registeredBase?: string | null;
  /** Origin of the page issuing the request; the same-origin fallback. */
  pageOrigin?: string | null;
}

export const resolveRequestUrl = (path: string, { registeredBase, pageOrigin }: RequestUrlInputs): string => {
  const base = (registeredBase || pageOrigin || "").replace(/\/+$/, "");
  return `${base}${path}`;
};

const HOSTED_UI = /\/ui(\/.*)?$/;

/**
 * Where a page's API lives when a LiteLLM gateway serves this UI under its own
 * `<root>/lens/ui/`, or null when Lens serves it at `/ui/` itself. The gateway
 * forwards everything under `<root>/lens/` to the Lens that built this UI.
 */
export const hostedApiRoot = (pagePath: string): string | null => {
  const root = pagePath.replace(HOSTED_UI, "");
  return root === "" || !HOSTED_UI.test(pagePath) ? null : root;
};

/** Page APIs sit under `/lens/` on Lens; the ones that predate that prefix have a `/lens/` twin. */
export const hostedApiPath = (apiRoot: string, path: string): string =>
  path === "/lens" || /^\/lens[/?]/.test(path) ? `${apiRoot}${path.slice("/lens".length)}` : `${apiRoot}${path}`;
