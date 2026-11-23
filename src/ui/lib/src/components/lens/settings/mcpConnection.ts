import { hostedApiPath } from "../../../lib/http/resolveApiBase";

export const MCP_DOCS_URL = "https://docs.litellm.ai/docs/proxy/lens/mcp";

export interface McpConnectionGuide {
  readonly url: string;
  readonly credential: string;
  readonly claudeCode: string;
  readonly codex: string;
}

export function mcpConnectionGuide(origin: string, hostedRoot: string | null, embedded: boolean): McpConnectionGuide {
  const throughGateway = embedded || hostedRoot !== null;
  const path = hostedRoot === null ? "/lens/mcp" : hostedApiPath(hostedRoot, "/lens/mcp");
  const url = `${origin.replace(/\/+$/, "")}${path}`;
  const variable = throughGateway ? "LITELLM_API_KEY" : "LENS_ADMIN_TOKEN";
  return {
    url,
    credential: throughGateway
      ? "Use your LiteLLM key. It reads only the traces you and your teams may see."
      : "Use LENS_ADMIN_TOKEN from the Lens deployment. It reads every trace. A tracing key is rejected.",
    claudeCode: `claude mcp add --transport http lens ${url} \\\n  --header "Authorization: Bearer $${variable}"`,
    codex: `codex mcp add lens --url ${url} --bearer-token-env-var ${variable}`,
  };
}
