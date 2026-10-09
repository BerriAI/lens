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
