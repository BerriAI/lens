export type ConnectionAuth = "bearer" | "none";

export function connectionProfile(name: string, auth: ConnectionAuth): string {
  return `[tool.lens.connections.${JSON.stringify(name)}]
base_url_env = "AGENT_BASE_URL"
auth = "${auth}"
${auth === "bearer" ? 'token_env = "AGENT_API_KEY"\n' : ""}`;
}

export function agentOrigin(
  input: string,
):
  | { readonly ok: true; readonly value: string }
  | { readonly ok: false; readonly error: string } {
  try {
    const url = new URL(input);
    if (
      !["http:", "https:"].includes(url.protocol) ||
      url.username ||
      url.password ||
      url.pathname !== "/" ||
      url.search ||
      url.hash
    )
      return {
        ok: false,
        error:
          "Use an HTTP(S) origin without a path, credentials, query or fragment",
      };
    return { ok: true, value: url.origin };
  } catch {
    return {
      ok: false,
      error: "Enter the agent origin, such as https://agent.example.com",
    };
  }
}

const shellString = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;

export function namedRunCommand(name: string, origin?: string): string {
  return `${origin ? `export AGENT_BASE_URL=${shellString(origin)}\n` : ""}uv run lens eval --name ${shellString(name)}`;
}
