import type { Sample } from "./evidence.js";
export interface Answer {
  readonly title: string;
  readonly summary: string;
  readonly sources: readonly { label: string; url: string }[];
  readonly opportunities?: readonly Opportunity[];
}
export type Category = "Performance" | "Agent quality" | "Reliability";
export interface Frequency {
  readonly category: Category;
  readonly label: string;
  readonly count?: number;
  readonly total?: number;
  readonly title?: string;
  readonly unit?: string;
  readonly affected_trace_ids?: readonly string[];
}
export interface MeasuredFrequency {
  readonly count: number;
  readonly total: number;
  readonly label: string;
  readonly title: string;
  readonly unit: string;
  readonly support?: boolean;
}
export interface Opportunity {
  readonly category: Category;
  readonly summary: string;
  readonly impact: number;
  readonly frequency_metric: string;
  readonly metric_title?: string;
  readonly sources: readonly { label: string; url: string }[];
  readonly frequency?: MeasuredFrequency;
}

export interface Turn {
  readonly role: "user" | "assistant";
  readonly content: string;
}

export interface FindingContext {
  readonly title: string;
  readonly issue: string;
  readonly traces: (Sample["traces"][number] & { readonly span_id?: string })[];
}
export interface AgentResponse {
  readonly answer: Answer;
  readonly evidenceUrls: readonly string[];
  readonly frequencies: Readonly<Record<string, Frequency>>;
  readonly detailed: boolean;
}
