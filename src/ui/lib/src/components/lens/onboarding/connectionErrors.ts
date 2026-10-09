import { ApiError } from "../../../lib/http/client";

export const connectionAuthStatus = (error: unknown): 401 | 403 | null =>
  error instanceof ApiError && (error.status === 401 || error.status === 403) ? error.status : null;
