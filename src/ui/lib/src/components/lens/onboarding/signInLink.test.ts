import { describe, expect, it, vi } from "vitest";
import { takeSignInToken } from "./signInLink";

const visit = (url: string) => {
  const { hash, pathname, search } = new URL(url);
  const history = { state: { page: 1 }, replaceState: vi.fn() };
  return {
    history,
    token: takeSignInToken({ hash, pathname, search }, history),
  };
};

describe("takeSignInToken", () => {
  it.each([
    ["a plain token", "#token=abc123", "abc123"],
    ["an encoded token", "#token=setup-token+%26%23", "setup-token &#"],
  ])("returns %s and removes it from the address", (_, fragment, expected) => {
    const { history, token } = visit(
      `http://localhost:4318/ui/?tab=traces${fragment}`,
    );
    expect(token).toBe(expected);
    expect(history.replaceState).toHaveBeenCalledExactlyOnceWith(
      { page: 1 },
      "",
      "/ui/?tab=traces",
    );
  });

  it.each([
    ["no fragment", ""],
    ["an unrelated fragment", "#section"],
    ["an empty token", "#token="],
    ["a token in the query string", "?token=abc123"],
  ])("leaves the address alone with %s", (_, suffix) => {
    const { history, token } = visit(`http://localhost:4318/ui/${suffix}`);
    expect(token).toBeNull();
    expect(history.replaceState).not.toHaveBeenCalled();
  });
});
