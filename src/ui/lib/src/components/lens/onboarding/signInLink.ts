export function takeSignInToken(
  location: Pick<Location, "hash" | "pathname" | "search">,
  history: Pick<History, "replaceState" | "state">,
): string | null {
  const token = new URLSearchParams(location.hash.slice(1)).get("token");
  if (!token) return null;
  history.replaceState(history.state, "", location.pathname + location.search);
  return token;
}
