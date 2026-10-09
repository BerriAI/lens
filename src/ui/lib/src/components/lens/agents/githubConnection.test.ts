import { describe, expect, it } from "vitest";
import { gitHubAuthorizationDestination } from "./githubConnection";

describe("GitHub authorization destinations", () => {
  it.each([
    ["https://github.com/apps/lens-qa/installations/new?state=qa", undefined],
    ["https://github.com/login/oauth/authorize?state=qa", null],
    ["https://GITHUB.com:443/apps/lens-qa/installations/new", undefined],
    [
      "https://connections.example.test/authorize?state=qa",
      "https://connections.example.test",
    ],
    [
      "https://connections.example.test:9443/authorize",
      "https://connections.example.test:9443",
    ],
    ["http://localhost:3106/authorize", "http://localhost:3106"],
    ["http://127.0.0.1:3106/authorize", "http://127.0.0.1:3106"],
    ["http://[::1]:3106/authorize", "http://[::1]:3106"],
  ])("allows %s with configured service %s", (destination, serviceOrigin) => {
    expect(gitHubAuthorizationDestination(destination!, serviceOrigin)).toBe(
      new URL(destination!).href,
    );
  });

  it.each([
    ["https://untrusted.example.test/authorize", undefined],
    ["https://github.com.untrusted.example.test/authorize", undefined],
    ["https://api.github.com/authorize", undefined],
    ["http://github.com/apps/lens-qa/installations/new", undefined],
    ["https://github.com:9443/authorize", undefined],
    ["https://user:password@github.com/authorize", undefined],
    ["https://connections.example.test/authorize", undefined],
    [
      "https://connections.example.test.untrusted.example.test/authorize",
      "https://connections.example.test",
    ],
    [
      "https://subdomain.connections.example.test/authorize",
      "https://connections.example.test",
    ],
    [
      "https://connections.example.test:9443/authorize",
      "https://connections.example.test",
    ],
    [
      "http://connections.example.test/authorize",
      "http://connections.example.test",
    ],
    ["http://127.0.0.1:3107/authorize", "http://127.0.0.1:3106"],
    [
      "http://localhost.untrusted.example.test/authorize",
      "http://localhost.untrusted.example.test",
    ],
    [
      "https://user:password@connections.example.test/authorize",
      "https://connections.example.test",
    ],
    [
      "https://connections.example.test/authorize",
      "https://user:password@connections.example.test",
    ],
    ["javascript:alert(1)", "https://connections.example.test"],
    ["data:text/html,untrusted", undefined],
    ["//github.com/apps/lens-qa/installations/new", undefined],
    ["/authorize", "https://connections.example.test"],
  ])("rejects %s with configured service %s", (destination, serviceOrigin) => {
    expect(() =>
      gitHubAuthorizationDestination(destination!, serviceOrigin),
    ).toThrow();
  });
});
