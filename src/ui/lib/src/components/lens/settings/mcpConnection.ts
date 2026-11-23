import { hostedApiPath } from "../../../lib/http/resolveApiBase";

export const MCP_DOCS_URL = "https://docs.litellm.ai/docs/proxy/lens/mcp";

export interface McpConnectionGuide {
  readonly id: "gateway" | "direct";
  readonly heading: string;
  readonly url: string;
  readonly credential: string;
  readonly claudeCode: string;
  readonly codex: string;
}

const guide = (
  id: McpConnectionGuide["id"],
  heading: string,
  url: string,
  variable: string,
  credential: string,
): McpConnectionGuide => ({
  id,
  heading,
  url,
  credential,
  claudeCode: `claude mcp add --transport http lens ${url} \\\n  --header "Authorization: Bearer $${variable}"`,
  codex: `codex mcp add lens --url ${url} --bearer-token-env-var ${variable}`,
});

/** Both ways to reach the MCP, the one this page is served through first. The other address is not known to the page. */
export function mcpConnectionGuides(
  origin: string,
  hostedRoot: string | null,
  embedded: boolean,
): readonly McpConnectionGuide[] {
  const throughGateway = embedded || hostedRoot !== null;
  const path = hostedRoot === null ? "/lens/mcp" : hostedApiPath(hostedRoot, "/lens/mcp");
  const here = `${origin.replace(/\/+$/, "")}${path}`;
  const gateway = guide(
    "gateway",
    "Through LiteLLM",
    throughGateway ? here : "https://<your-litellm-gateway>/lens/mcp",
    "LITELLM_API_KEY",
    "Use your own LiteLLM key, no admin token needed. It reads only the traces you and your teams may see.",
  );
  const direct = guide(
    "direct",
    "Directly to Lens",
    throughGateway ? "https://<your-lens-address>/lens/mcp" : here,
    "LENS_ADMIN_TOKEN",
    "Use LENS_ADMIN_TOKEN from the Lens deployment. It reads every trace. A tracing key is rejected.",
  );
  return throughGateway ? [gateway, direct] : [direct, gateway];
}
