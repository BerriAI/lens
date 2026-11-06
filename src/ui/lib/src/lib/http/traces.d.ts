/* Generated from schema/lens-traces.json. Run npm run generate:trace-contract. */
export type UIContent = UIMessages | UIFields | UIText;
export type ChatRole = "system" | "user" | "assistant" | "tool";

export interface TraceConversationPage {
  turns: TraceConversationTurn[];
  next_cursor: string | null;
  [k: string]: unknown;
}
export interface TraceConversationTurn {
  trace_id: string;
  trace_ref: string;
  span_id: string;
  start_time: string;
  input: string;
  output: string;
  input_ui: UIContent;
  output_ui: UIContent;
  [k: string]: unknown;
}
export interface UIMessages {
  messages: UIMessage[];
  kind: "messages";
  [k: string]: unknown;
}
export interface UIMessage {
  role: ChatRole;
  content: string;
  name?: string | null;
  tool_calls?: UIToolCall[] | null;
  [k: string]: unknown;
}
export interface UIToolCall {
  name: string;
  arguments: string;
  [k: string]: unknown;
}
export interface UIFields {
  fields: UIField[];
  kind: "fields";
  [k: string]: unknown;
}
export interface UIField {
  key: string;
  value: string;
  [k: string]: unknown;
}
export interface UIText {
  text: string;
  kind: "text";
  [k: string]: unknown;
}
export interface TraceConversationRequest {
  trace_ref?: string;
  cursor?: string | null;
  page_size?: number | null;
  [k: string]: unknown;
}
export interface LogListRequest {
  start_ms?: number | null;
  end_ms?: number | null;
  service?: string | null;
  event?: string | null;
  severity?: string | null;
  trace_id?: string | null;
  conversation_id?: string | null;
  cursor?: string | null;
  page_size?: number | null;
  [k: string]: unknown;
}
export interface OtelLogPage {
  data: OtelLogSummary[];
  next_cursor: string | null;
  [k: string]: unknown;
}
export interface OtelLogSummary {
  record_id: string;
  timestamp_ns: string;
  observed_timestamp_ns: string;
  severity_number: number;
  severity_text: string;
  event_name: string;
  semantic_event_name: string | null;
  scope_name: string;
  service_name: string;
  trace_id: string;
  span_id: string;
  conversation_id: string | null;
  [k: string]: unknown;
}
export type LogValue =
  | {
      type: "null";
      [k: string]: unknown;
    }
  | {
      type: "string";
      value: string;
      [k: string]: unknown;
    }
  | {
      type: "boolean";
      value: boolean;
      [k: string]: unknown;
    }
  | {
      type: "integer";
      value: number;
      [k: string]: unknown;
    }
  | {
      type: "double";
      value: number;
      [k: string]: unknown;
    }
  | {
      type: "bytes";
      value: number[];
      [k: string]: unknown;
    }
  | {
      type: "array";
      value: LogValue[];
      [k: string]: unknown;
    }
  | {
      type: "map";
      value: {
        [k: string]: LogValue;
      };
      [k: string]: unknown;
    };
export type CodexEventKind =
  | "conversation_starts"
  | "startup_phase"
  | "user_prompt"
  | "agent_response"
  | "tool_result"
  | "tool_decision"
  | "sandbox_outcome"
  | "api_request"
  | "sse_event"
  | "turn_ttft"
  | "turn_cost"
  | "skill_invocation"
  | "guardian_assessment"
  | "websocket"
  | "plugin"
  | "other";
export type CodexMessageRole = "user" | "assistant" | "tool";
export type CodexContentState = "present" | "redacted" | "absent" | "malformed";

export interface OtelLogRecord {
  record_id: string;
  timestamp_ns: string;
  raw_timestamp_ns: string;
  observed_timestamp_ns: string;
  severity_number: number;
  severity_text: string;
  event_name: string;
  semantic_event_name: string | null;
  body: LogValue | null;
  attributes: {
    [k: string]: LogValue;
  };
  resource_attributes: {
    [k: string]: LogValue;
  };
  resource_schema_url: string;
  scope_name: string;
  scope_version: string;
  scope_attributes: {
    [k: string]: LogValue;
  };
  scope_schema_url: string;
  trace_id: string;
  span_id: string;
  trace_flags: number;
  dropped_attributes_count: number;
  service_name: string;
  conversation_id: string | null;
  codex: CodexProjection | null;
  [k: string]: unknown;
}
export interface CodexProjection {
  event_kind: CodexEventKind;
  message_role: CodexMessageRole | null;
  content_state: CodexContentState | null;
  content: string | null;
  conversation_id: string | null;
  turn_id: string | null;
  item_id: string | null;
  call_id: string | null;
  tool_name: string | null;
  success: boolean | null;
  duration_ms: number | null;
  truncated: boolean | null;
  [k: string]: unknown;
}
