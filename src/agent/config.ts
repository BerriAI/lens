export interface AgentConfig {
  readonly agent: string;
  readonly service?: string;
  readonly publicUrl: string;
  readonly apiUrl: string;
  readonly lensKey: string;
  readonly openaiKey: string;
  readonly openaiBaseUrl: string;
  readonly model: string;
}
