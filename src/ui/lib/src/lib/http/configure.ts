import {
  registerAuthHeaderNameGetter,
  registerAuthTokenGetter,
  registerBaseUrlGetter,
  registerErrorHandler,
} from "./runtime";
import { setServerRootPath } from "../serverRootPath";

export interface LensHttpConfig {
  readonly getBaseUrl: () => string;
  readonly getAuthToken: () => string | null;
  readonly getAuthHeaderName?: () => string;
  readonly onError?: (message: string) => void;
  readonly serverRootPath?: string;
  readonly getServerRootPath?: () => string;
}

export function configureLensHttp(config: LensHttpConfig): void {
  registerBaseUrlGetter(config.getBaseUrl);
  registerAuthTokenGetter(config.getAuthToken);
  registerAuthHeaderNameGetter(
    config.getAuthHeaderName ?? (() => "Authorization"),
  );
  registerErrorHandler(config.onError ?? (() => {}));
  setServerRootPath(config.getServerRootPath ?? config.serverRootPath ?? "/");
}
