import { vi } from "vitest";
import { readRequest } from "./lens-test-utils";
import type {
  AgentConversation,
  AgentReport,
  WorkspaceAgent,
} from "../src/lib/http/agentWorkspace";

export const admin = {
  accessToken: "test-workspace-token",
  userRole: "Admin",
  readOnly: false,
};
export const main: WorkspaceAgent = {
  id: "main-agent",
  model: null,
  name: "Lens",
  instructions: "",
  runtime: "deepagents",
  enabled: true,
  created_at: 1,
};
export const bot: WorkspaceAgent = {
  id: "review-bot",
  model: "bot-model",
  name: "Request reviewer",
  instructions: "Review new requests",
  runtime: "hermes",
  enabled: true,
  created_at: 1,
};
export const chat: AgentConversation = {
  id: "saved-chat",
  model: "session-model",
  actor_id: main.id,
  title: "Investigate failed requests",
  native_id: "native-session",
  permanent: false,
  created_at: 1,
  updated_at: 1,
};
export const botChat: AgentConversation = {
  ...chat,
  id: "bot-chat",
  actor_id: bot.id,
  title: "Bot Chat",
  permanent: true,
};
export const report: AgentReport = {
  id: "outcomes",
  title: "Three requests need attention",
  markdown: "Full findings and evidence",
  preview: false,
  producer_id: bot.id,
  execution_id: "run-1",
  updated_at: 1,
  traces: [{ trace_id: "trace-1", trace_ref: "scope-1" }],
  artifacts: [],
  artifact_ids: [],
};

export function workspaceNetwork(writable = true) {
  return vi.fn<typeof fetch>(async (input, init) => {
    const request = await readRequest(input, init);
    const { path } = request;
    if (path === "/lens/workspace/models")
      return Response.json(["test-model", "second-model"]);
    if (path === "/lens/workspace/activity") return Response.json([]);
    if (path === "/lens/workspace")
      return Response.json({
        agents: [main, bot],
        model: "test-model",
        writable,
        slack_configured: false,
      });
    if (path === "/lens/workspace/conversations") {
      if (request.method === "POST") return Response.json(chat);
      return Response.json(
        request.query.get("actor_id") === bot.id ? [botChat] : [chat],
      );
    }
    if (
      path === "/lens/workspace/conversations/" + chat.id ||
      path === "/lens/workspace/conversations/" + botChat.id
    )
      return Response.json({
        conversation: path.endsWith(botChat.id) ? botChat : chat,
        turns: [],
        events: [],
        native_history: null,
      });
    if (path.endsWith("/messages"))
      return Response.json({ id: "turn-1", status: "queued" });
    if (path === "/lens/workspace/reports") return Response.json([report]);
    if (path === "/lens/workspace/reports/" + report.id)
      return Response.json(report);
    if (path === "/lens/workspace/annotations/query")
      return Response.json({ annotations: [], next_cursor: null });
    if (path === "/v1/traces/agents") return Response.json({ agents: [] });
    if (path === "/v1/traces")
      return Response.json({ data: [], next_cursor: null });
    if (path === "/lens")
      return Response.json({ lenses: [], workers: [], tracing_enabled: true });
    if (path === "/lens/admin/activity/available")
      return Response.json({ traces: false, requests: false });
    return Response.json({ data: [], traces: false });
  });
}
