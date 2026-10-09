import { createAppAuth } from "@octokit/auth-app";
import { z } from "zod";
import { redact } from "./investigator-evidence.js";

const treeSchema = z.object({
  truncated: z.boolean(),
  tree: z.array(
    z.object({
      path: z.string(),
      type: z.string(),
      sha: z.string(),
      size: z.number().optional(),
    }),
  ),
});
export interface CodeFile {
  readonly path: string;
  readonly content: string;
  readonly url: string;
}
export class Repository {
  readonly files = new Map<string, CodeFile>();
  private token = "";
  sha = "";
  private paths: z.infer<typeof treeSchema>["tree"] = [];
  constructor(
    readonly name: string,
    private readonly clientId: string,
    private readonly privateKey: string,
    private readonly fetcher: typeof fetch = fetch,
  ) {
    if (!/^[a-zA-Z0-9_.-]+\/[a-zA-Z0-9_.-]+$/.test(name))
      throw new Error("Invalid allowed repository");
  }
  private async request(
    path: string,
    token: string,
    signal: AbortSignal,
    body?: unknown,
  ): Promise<unknown> {
    const response = await this.fetcher(`https://api.github.com${path}`, {
      method: body ? "POST" : "GET",
      headers: {
        Authorization: `Bearer ${token}`,
        Accept: "application/vnd.github+json",
        "User-Agent": "lens-investigator",
        "X-GitHub-Api-Version": "2026-03-10",
        ...(body ? { "Content-Type": "application/json" } : {}),
      },
      ...(body ? { body: JSON.stringify(body) } : {}),
      signal: AbortSignal.any([signal, AbortSignal.timeout(15_000)]),
      redirect: "error",
    });
    if (!response.ok || !response.body)
      throw new Error("Repository read unavailable");
    const chunks: Uint8Array[] = [];
    let length = 0;
    for await (const chunk of response.body) {
      length += chunk.length;
      if (length > 2_000_000)
        throw new Error("Repository response exceeds limit");
      chunks.push(chunk);
    }
    return JSON.parse(Buffer.concat(chunks).toString("utf8"));
  }
  async initialize(signal: AbortSignal): Promise<void> {
    const auth = createAppAuth({
      appId: this.clientId,
      privateKey: this.privateKey,
    });
    const jwt = (await auth({ type: "app" })).token;
    const installation = z
      .object({ id: z.number().int() })
      .parse(
        await this.request(`/repos/${this.name}/installation`, jwt, signal),
      );
    this.token = z.object({ token: z.string() }).parse(
      await this.request(
        `/app/installations/${installation.id}/access_tokens`,
        jwt,
        signal,
        {
          repositories: [this.name.split("/")[1]],
          permissions: { contents: "read" },
        },
      ),
    ).token;
    const repo = z
      .object({ default_branch: z.string() })
      .parse(await this.request(`/repos/${this.name}`, this.token, signal));
    this.sha = z
      .object({ sha: z.string().regex(/^[a-f0-9]{40}$/) })
      .parse(
        await this.request(
          `/repos/${this.name}/commits/${encodeURIComponent(repo.default_branch)}`,
          this.token,
          signal,
        ),
      ).sha;
    const tree = treeSchema.parse(
      await this.request(
        `/repos/${this.name}/git/trees/${this.sha}?recursive=1`,
        this.token,
        signal,
      ),
    );
    this.paths = tree.tree.filter(
      (item) =>
        item.type === "blob" &&
        (item.size ?? Infinity) <= 50_000 &&
        /\.(rs|ts|tsx|js|py|md|toml|json)$/.test(item.path) &&
        !/(^|\/)(node_modules|vendor|\.git|\.env)|lock\.(json|toml)$|package-lock\.json$/.test(
          item.path,
        ),
    );
  }
  list(query: string): string {
    const paths = this.paths
      .filter((item) => item.path.toLowerCase().includes(query.toLowerCase()))
      .slice(0, 80)
      .map((item) => item.path);
    return JSON.stringify({
      repository: this.name,
      commit: this.sha,
      paths,
      bounded: true,
      note: "Current repository code, not verified deployed code",
    });
  }
  async read(path: string, signal: AbortSignal): Promise<string> {
    const item = this.paths.find((item) => item.path === path);
    if (!item || this.files.size >= 6)
      return "File unavailable or per-investigation read limit reached";
    const blob = z
      .object({ content: z.string(), encoding: z.literal("base64") })
      .parse(
        await this.request(
          `/repos/${this.name}/git/blobs/${item.sha}`,
          this.token,
          signal,
        ),
      );
    const content = redact(
      Buffer.from(blob.content, "base64").toString("utf8"),
    ).slice(0, 16_000);
    const url = `https://github.com/${this.name}/blob/${this.sha}/${path.split("/").map(encodeURIComponent).join("/")}`;
    this.files.set(path, { path, content, url });
    return JSON.stringify({
      path,
      commit: this.sha,
      url,
      content,
      truncated: (item.size ?? 0) > Buffer.byteLength(content),
      untrusted: true,
    });
  }
}
