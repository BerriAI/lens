import { createHash } from "node:crypto";
import { z } from "zod";

export const digest = (value: string): string =>
  createHash("sha256").update(value).digest("hex");

export function clickhouseConnection(env: NodeJS.ProcessEnv): string {
  if (env.CLICKHOUSE_URL?.trim()) return env.CLICKHOUSE_URL;
  const url = new URL("http://localhost:8123");
  if (!env.CLICKHOUSE_HOST) return url.toString();
  if (!env.CLICKHOUSE_PASSWORD) throw new Error("Missing ClickHouse password");
  url.hostname = env.CLICKHOUSE_HOST;
  url.username = env.CLICKHOUSE_USER || "default";
  url.password = env.CLICKHOUSE_PASSWORD;
  return url.toString();
}
const headSchema = z.object({
  key: z.string(),
  revision: z.coerce.number().int().nonnegative().safe(),
  digest: z.string(),
});
export interface Snapshot<T> {
  readonly key: string;
  readonly revision: number;
  readonly digest: string;
  readonly value: T;
}
export interface StateStore<T> {
  read(): Promise<Snapshot<T>>;
  commit(previous: Snapshot<T>, value: T): Promise<Snapshot<T>>;
}

export class ClickHouseState<T> implements StateStore<T> {
  private readonly url: URL;
  private readonly authorization: string;
  constructor(
    connection: string,
    database: string,
    private readonly key: string,
    private readonly schema: z.ZodType<T>,
    private readonly initial: T,
    private readonly fetcher: typeof fetch = fetch,
  ) {
    this.url = new URL(connection);
    if (
      !/^https?:$/.test(this.url.protocol) ||
      !/^[a-zA-Z_][a-zA-Z0-9_]*$/.test(database)
    )
      throw new Error("Invalid investigator state connection");
    this.authorization = `Basic ${Buffer.from(`${decodeURIComponent(this.url.username) || "default"}:${decodeURIComponent(this.url.password)}`).toString("base64")}`;
    this.url.username = "";
    this.url.password = "";
    this.url.search = "";
    this.url.searchParams.set("database", database);
  }
  private async command(
    query: string,
    parameters: Record<string, string> = {},
    body = "",
  ): Promise<string> {
    const url = new URL(this.url);
    for (const [key, value] of Object.entries({
      query,
      keeper_map_strict_mode: "1",
      insert_keeper_max_retries: "0",
      async_insert: "0",
      wait_end_of_query: "1",
      send_progress_in_http_headers: "0",
      output_format_json_quote_64bit_integers: "0",
    }))
      url.searchParams.set(key, value);
    for (const [key, value] of Object.entries(parameters))
      url.searchParams.set(
        `param_${key}`,
        value
          .replace(/\\/g, "\\\\")
          .replace(/\t/g, "\\t")
          .replace(/\n/g, "\\n")
          .replace(/\r/g, "\\r")
          .replace(/\0/g, "\\0"),
      );
    const response = await this.fetcher(url, {
      method: "POST",
      headers: { Authorization: this.authorization },
      body,
      redirect: "error",
      signal: AbortSignal.timeout(15_000),
    });
    const code = response.headers.get("x-clickhouse-exception-code");
    if (!response.ok || (code && code !== "0") || !response.body)
      throw new Error("Investigator state read or write failed");
    const chunks: Uint8Array[] = [];
    let length = 0;
    for await (const chunk of response.body) {
      length += chunk.length;
      if (length > 1_048_576) throw new Error("State response too large");
      chunks.push(chunk);
    }
    return Buffer.concat(chunks).toString("utf8");
  }
  async read(): Promise<Snapshot<T>> {
    const text = await this.command(
      "SELECT key, revision, digest FROM lens_state_heads WHERE key={key:String} FORMAT JSONEachRow",
      { key: this.key },
    );
    if (!text.trim())
      return { key: this.key, revision: 0, digest: "", value: this.initial };
    const head = headSchema.parse(JSON.parse(text));
    if (
      head.key !== this.key ||
      (head.revision > 0 && !/^[a-f0-9]{64}$/.test(head.digest))
    )
      throw new Error("Invalid investigator state head");
    if (head.revision === 0 && head.digest === "")
      return { ...head, value: this.initial };
    const body = await this.command(
      "SELECT data FROM lens_state_blobs FINAL WHERE key={key:String} AND revision={revision:UInt64} AND digest={digest:String} FORMAT JSONEachRow",
      { key: this.key, revision: String(head.revision), digest: head.digest },
    );
    const blob = z.object({ data: z.string() }).parse(JSON.parse(body));
    if (digest(blob.data) !== head.digest)
      throw new Error("Invalid investigator state digest");
    return { ...head, value: this.schema.parse(JSON.parse(blob.data)) };
  }
  async commit(previous: Snapshot<T>, value: T): Promise<Snapshot<T>> {
    if (previous.key !== this.key)
      throw new Error("Invalid investigator state scope");
    if (previous.revision === 0) {
      try {
        await this.command(
          "INSERT INTO lens_state_heads FORMAT JSONEachRow",
          {},
          JSON.stringify({ key: this.key, revision: 0, digest: "" }),
        );
      } catch {
        const current = await this.read();
        if (current.revision !== 0)
          throw new Error("Investigator state conflict");
      }
    }
    const data = JSON.stringify(this.schema.parse(value));
    const head = {
      key: this.key,
      revision: previous.revision + 1,
      digest: digest(data),
    };
    await this.command(
      "INSERT INTO lens_state_blobs FORMAT JSONEachRow",
      {},
      JSON.stringify({ ...head, data }),
    );
    await this.command(
      "ALTER TABLE lens_state_heads UPDATE revision=revision+1+throwIf(revision!={revision:UInt64}, 'LENS_STATE_CONFLICT'), digest={digest:String} WHERE key={key:String}",
      {
        key: this.key,
        revision: String(previous.revision),
        digest: head.digest,
      },
    );
    const confirmed = await this.read();
    if (
      confirmed.revision !== head.revision ||
      confirmed.digest !== head.digest
    )
      throw new Error("Investigator state ownership changed");
    return confirmed;
  }
}
