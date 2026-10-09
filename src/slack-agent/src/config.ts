import { isIP } from "node:net";

export interface Config {
  readonly botToken: string;
  readonly appToken: string;
  readonly workspace: string;
  readonly channel: string;
  readonly agent: string;
  readonly service?: string;
  readonly publicUrl: string;
  readonly apiUrl: string;
  readonly lensKey: string;
  readonly openaiKey: string;
  readonly openaiBaseUrl: string;
  readonly model: string;
}

function required(env: NodeJS.ProcessEnv, name: string): string {
  const value = env[name]?.trim();
  if (!value) throw new Error(`Missing ${name}`);
  return value;
}

export function localApiUrl(listener = "0.0.0.0:4318"): string {
  const address = new URL(`http://${listener}`);
  const host = address.hostname.replace(/^\[|\]$/g, "");
  if (
    !isIP(host) ||
    !address.port ||
    address.username ||
    address.password ||
    address.pathname !== "/"
  ) {
    throw new Error("Invalid LITELLM_LENS_LISTEN");
  }
  const loopback =
    host === "0.0.0.0"
      ? "127.0.0.1"
      : host === "::"
        ? "[::1]"
        : address.hostname;
  return `http://${loopback}:${address.port}`;
}

function endpoint(value: string, name: string, loopback = false): string {
  const url = new URL(value);
  if (
    url.username ||
    url.password ||
    url.search ||
    url.hash ||
    (url.protocol !== "https:" &&
      !(
        loopback &&
        url.protocol === "http:" &&
        ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname)
      ))
  ) {
    throw new Error(`Invalid ${name}`);
  }
  return url.toString().replace(/\/$/, "");
}

export function configFrom(env: NodeJS.ProcessEnv): Config | undefined {
  if (env.LENS_SLACK_CHAT_ENABLED !== "true") return undefined;
  if (env.LENS_SLACK_ALL_TEAMS !== "true" || env.LENS_SLACK_TEAM_ID?.trim()) {
    throw new Error(
      "Slack chat currently requires explicit LENS_SLACK_ALL_TEAMS=true without LENS_SLACK_TEAM_ID",
    );
  }
  const workspace = required(env, "LENS_SLACK_WORKSPACE_ID");
  const channel = required(env, "LENS_SLACK_CHANNEL_ID");
  if (!/^T[A-Z0-9]+$/.test(workspace) || !/^[CG][A-Z0-9]+$/.test(channel)) {
    throw new Error("Invalid Slack workspace or channel ID");
  }
  return {
    botToken: required(env, "LENS_SLACK_BOT_TOKEN"),
    appToken: required(env, "LENS_SLACK_APP_TOKEN"),
    workspace,
    channel,
    agent: required(env, "LENS_SLACK_AGENT"),
    service:
      env.LENS_SLACK_SERVICE?.trim() || required(env, "LENS_SLACK_AGENT"),
    publicUrl: endpoint(required(env, "LENS_PUBLIC_URL"), "LENS_PUBLIC_URL"),
    apiUrl: localApiUrl(env.LITELLM_LENS_LISTEN),
    lensKey:
      env.LENS_SLACK_LENS_API_KEY?.trim() || required(env, "LENS_ADMIN_TOKEN"),
    openaiKey: required(env, "OPENAI_API_KEY"),
    openaiBaseUrl: endpoint(
      env.OPENAI_BASE_URL || "https://api.openai.com/v1",
      "OPENAI_BASE_URL",
      true,
    ),
    model: env.LENS_SLACK_MODEL?.trim() || "gpt-6-luna",
  };
}
