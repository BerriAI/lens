import type * as Wire from "../../../lib/http/worker";
import type { components } from "../../../lib/http/schema";

export type Lens = components["schemas"]["Lens"];

export type Settings = components["schemas"]["LensSettings"];

export type LensList = components["schemas"]["LensList"];
export type SignalConfig = components["schemas"]["SignalConfig"];
export type Signal = NonNullable<SignalConfig["signals"]>[number];

export type Finding = components["schemas"]["Finding"];

export type Sample = components["schemas"]["Sample"];

export type Job = components["schemas"]["Job"];

export type IssueBrief = Wire.IssueBrief;

export type ActivitySelection = Pick<Settings, "source"> &
  Partial<
    Pick<
      Settings,
      | "service"
      | "agent_name"
      | "filters"
      | "lookback_hours"
      | "sample_percent"
      | "sample_size"
      | "team_id"
      | "execution_ids"
    >
  >;

export type Worker = LensList["workers"][number];

export type Review = components["schemas"]["Review"];

export type InFlight = Wire.InFlight;

export type Activity = components["schemas"]["Activity"];

export type ToolCount = Wire.ToolCount;

export type ReviewVerdict = Wire.ReviewVerdict;

export interface RunWindow {
  agent_name?: string;
  start?: string;
  end?: string;
  lookback_hours?: number;
}

export interface AnalysisModelInfo {
  model_group: string;
  providers: string[];
  mode?: string | null;
  supported_openai_params?: string[] | null;
}

export interface GatewayStatus {
  readonly configured: boolean;
  readonly connected: boolean;
  readonly api_base: string | null;
  readonly analysis_models: number;
  readonly evaluation_models: number;
  readonly error: string | null;
  readonly last_refreshed: string | null;
}
