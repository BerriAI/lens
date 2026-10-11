import assert from "node:assert/strict";
import { test } from "node:test";
import { Repository } from "./repository.js";

test("public source reads pin a commit, include MDX, and send no installation credentials", async () => {
  const sha = "a".repeat(40);
  const requests: string[] = [];
  const repo = new Repository(
    "example/docs",
    "unused",
    "unused",
    async (url, init) => {
      assert.equal(new Headers(init?.headers).has("Authorization"), false);
      assert.equal(init?.method, "GET");
      const path = new URL(String(url)).pathname;
      requests.push(path);
      const value = path.endsWith("/docs")
        ? { default_branch: "default", private: false }
        : path.endsWith("/commits/default")
          ? { sha }
          : path.endsWith(`/trees/${sha}`)
            ? {
                truncated: false,
                tree: [
                  {
                    type: "blob",
                    path: "docs/setup.mdx",
                    size: 30,
                    sha: "blob-sha",
                  },
                ],
              }
            : path.endsWith("/blobs/blob-sha")
              ? {
                  encoding: "base64",
                  content: Buffer.from(
                    "Follow these setup instructions",
                  ).toString("base64"),
                }
              : assert.fail(`Unexpected source request ${path}`);
      return new Response(JSON.stringify(value));
    },
    "public",
  );
  await repo.initialize(AbortSignal.timeout(5000));
  assert(repo.list("setup").includes("docs/setup.mdx"));
  const file = JSON.parse(
    await repo.read("docs/setup.mdx", AbortSignal.timeout(5000)),
  );
  assert.equal(file.commit, sha);
  assert(file.url.includes(`/blob/${sha}/docs/setup.mdx`));
  assert.equal(file.content, "Follow these setup instructions");
  assert.equal(requests.length, 4);
  assert.equal(
    await repo.read("docs/unlisted.md", AbortSignal.timeout(5000)),
    "File unavailable or per-investigation read limit reached",
  );
  assert.equal(requests.length, 4);
});

test("public mode refuses private or inaccessible repositories without falling back to app access", async () => {
  for (const response of [
    new Response("{}", { status: 404 }),
    new Response(JSON.stringify({ default_branch: "main", private: true })),
  ]) {
    let requests = 0;
    const repo = new Repository(
      "example/private",
      "unused",
      "unused",
      async (_url, init) => {
        requests++;
        assert.equal(new Headers(init?.headers).has("Authorization"), false);
        return response;
      },
      "public",
    );
    await assert.rejects(repo.initialize(AbortSignal.timeout(5000)));
    assert.equal(requests, 1);
  }
});
