export interface WorkspaceAgent {
  id: string;
  name: string;
  instructions: string;
  runtime: "deepagents" | "hermes";
  enabled: boolean;
  created_at: number;
}

export interface AgentWorkspaceState {
  agents: WorkspaceAgent[];
  model: string;
  writable: boolean;
  slack_configured: boolean;
}

export interface AgentConversation {
  id: string;
  actor_id: string;
  title: string;
  native_id: string | null;
  permanent: boolean;
  created_at: number;
  updated_at: number;
}

export interface AgentTurn {
  id: string;
  conversation_id: string;
  input: string;
  status: "queued" | "running" | "completed" | "failed" | "interrupted";
  error: string | null;
  created_at: number;
  updated_at: number;
}

export interface AgentEvent {
  sequence: number;
  turn_id: string;
  kind: string;
  data: Record<string, unknown>;
  created_at: number;
}

export interface ConversationState {
  conversation: AgentConversation;
  turns: AgentTurn[];
  events: AgentEvent[];
  native_history: {
    messages: {
      id: string | number;
      role: "user" | "assistant";
      content: string;
      created_at: number | null;
    }[];
    truncated: boolean;
  } | null;
}

export interface ArtifactInfo {
  id: string;
  filename: string;
  media_type: string;
  size: number;
  sha256: string;
}
export interface AgentArtifact extends ArtifactInfo {
  data_base64: string;
}

export interface AgentReport {
  preview: boolean;
  artifacts: ArtifactInfo[];
  artifact_ids: string[];
  id: string;
  title: string;
  markdown: string;
  traces: { trace_id: string; trace_ref: string }[];
  producer_id: string;
  execution_id: string;
  updated_at: number;
}

export interface TraceAnnotation {
  definition_key: string;
  producer_name: string;
  trace: { trace_id: string; trace_ref: string };
  definition_id: string;
  value: string | number | boolean;
  evidence: string;
  producer_id: string;
  execution_id: string;
  updated_at: number;
}

type JsonResponse<T> = {
  responses: { 200: { content: { "application/json": T } } };
};
type JsonBody<T> = { requestBody: { content: { "application/json": T } } };
type IdPath = { parameters: { path: { id: string } } };

export interface AgentWorkspacePaths {
  "/lens/workspace": { get: JsonResponse<AgentWorkspaceState> };
  "/lens/workspace/bots": {
    post: JsonBody<{ name: string; instructions: string; id: string }> &
      JsonResponse<WorkspaceAgent>;
  };
  "/lens/workspace/bots/{id}": {
    patch: IdPath &
      JsonBody<{ enabled: boolean }> &
      JsonResponse<WorkspaceAgent>;
  };
  "/lens/workspace/conversations": {
    get: { parameters: { query: { actor_id: string } } } & JsonResponse<
      AgentConversation[]
    >;
    post: JsonBody<{ actor_id?: string; title: string; id?: string }> &
      JsonResponse<AgentConversation>;
  };
  "/lens/workspace/conversations/{id}": {
    get: {
      parameters: { path: { id: string }; query?: { after?: number } };
    } & JsonResponse<ConversationState>;
  };
  "/lens/workspace/conversations/{id}/messages": {
    post: IdPath &
      JsonBody<{ id: string; text: string }> &
      JsonResponse<AgentTurn>;
  };
  "/lens/workspace/conversations/{id}/slack": {
    get: IdPath &
      JsonResponse<{
        linked: boolean;
        channel_id?: string;
        sender_id?: string;
      }>;
    post: IdPath & JsonResponse<{ command: string; expires_at: number }>;
    delete: IdPath & JsonResponse<{ linked: boolean }>;
  };
  "/lens/workspace/conversations/{id}/stop": {
    post: IdPath & JsonResponse<{ stopped: boolean }>;
  };
  "/lens/workspace/annotations/query": {
    post: JsonBody<{
      trace_id?: string;
      trace_ref?: string;
      definition_id?: string;
      producer_id?: string;
      value?: string | boolean | number;
      after?: number;
      limit?: number;
    }> &
      JsonResponse<{
        annotations: TraceAnnotation[];
        next_cursor: number | null;
      }>;
  };
  "/lens/workspace/reports": {
    get: {
      parameters: {
        query?: { trace_id?: string; trace_ref?: string; before?: string };
      };
    } & JsonResponse<AgentReport[]>;
  };
  "/lens/workspace/artifacts/{id}": {
    get: IdPath & JsonResponse<AgentArtifact>;
  };
  "/lens/workspace/reports/{id}": { get: IdPath & JsonResponse<AgentReport> };
}
