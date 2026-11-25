export { LensWorkspace } from "./components/lens/LensWorkspace";
export { LensHostProvider, type LensHost, type SpendLogLookup, type SpendLogDrawerProps } from "./host/LensHost";
export {
  GATEWAY_FRAME_CHANNEL,
  onGatewayFrameCommand,
  useGatewayFrameTheme,
  type GatewayFrameAuth,
} from "./host/gatewayFrame";
export { configureLensHttp, type LensHttpConfig } from "./lib/http/configure";
export { hostedApiPath, hostedApiRoot } from "./lib/http/resolveApiBase";
export { takeSignInToken } from "./components/lens/onboarding/signInLink";
export { LensServicesProvider, liveLensServices, type LensServices } from "./components/lens/data/LensServices";
export type { LogEntry } from "./components/logs/types";
export { Toaster } from "./components/ui/sonner";
export type {
  SpanDetail,
  SpanErrorPage,
  SpanErrorQuery,
  SpanQuery,
  Trace,
  TraceDetailQuery,
  TraceListQuery,
  TracePage,
} from "./components/lens/traces/types";
