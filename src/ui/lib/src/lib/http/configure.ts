import {
  registerAuthHeaderNameGetter,
  registerAuthTokenGetter,
  registerBaseUrlGetter,
  registerErrorHandler,
  registerPathResolver,
  setRequestCredentials,
} from "./runtime";
import { setServerRootPath } from "../serverRootPath";

export interface LensHttpConfig {
  readonly getBaseUrl: () => string;
  readonly getAuthToken: () => string | null;
  readonly getAuthHeaderName?: () => string;
  readonly credentials?: RequestCredentials;
  readonly onError?: (message: string) => void;
  readonly serverRootPath?: string;
  readonly getServerRootPath?: () => string;
  /** Rewrites each API path before the base URL is applied, for a host that serves the API somewhere else. */
  readonly resolvePath?: (path: string) => string;
}

export function configureLensHttp(config: LensHttpConfig): void {
  registerBaseUrlGetter(config.getBaseUrl);
  registerAuthTokenGetter(config.getAuthToken);
  setRequestCredentials(config.credentials);
  registerAuthHeaderNameGetter(
    config.getAuthHeaderName ?? (() => "Authorization"),
  );
  registerErrorHandler(config.onError ?? (() => {}));
  registerPathResolver(config.resolvePath ?? ((path) => path));
  setServerRootPath(config.getServerRootPath ?? config.serverRootPath ?? "/");
}
