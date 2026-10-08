import { getAuthHeaderName } from "./runtime";

export const authHeaders = (token: string): Record<string, string> =>
  token ? { [getAuthHeaderName()]: `Bearer ${token}` } : {};
