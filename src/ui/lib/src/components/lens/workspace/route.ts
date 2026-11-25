"use client";

import {
  createParser,
  parseAsBoolean,
  parseAsString,
  parseAsStringLiteral,
  useQueryStates,
} from "nuqs";

import type { AnnotationSelection } from "../../../lib/http/agentWorkspace";

const annotationParser = createParser<AnnotationSelection>({
  parse: (text) => {
    try {
      const value: unknown = JSON.parse(text);
      if (
        value &&
        typeof value === "object" &&
        "producer_id" in value &&
        typeof value.producer_id === "string" &&
        "definition_id" in value &&
        typeof value.definition_id === "string" &&
        "value" in value &&
        (typeof value.value === "string" ||
          typeof value.value === "boolean" ||
          (typeof value.value === "number" && Number.isFinite(value.value)))
      )
        return {
          producer_id: value.producer_id,
          definition_id: value.definition_id,
          value: value.value,
        };
    } catch {}
    return null;
  },
  serialize: JSON.stringify,
});

export const WORKSPACE_TABS = [
  "home",
  "new_session",
  "bots",
  "traces",
] as const;
export type WorkspaceTab = (typeof WORKSPACE_TABS)[number];
const parsers = {
  tab: parseAsStringLiteral(WORKSPACE_TABS),
  chat: parseAsString,
  bot: parseAsString,
  report: parseAsString,
  annotations: annotationParser,
  demo: parseAsBoolean.withDefault(false),
  trace: parseAsString,
};

export function useAgentWorkspaceRoute() {
  const [route, setRoute] = useQueryStates(parsers, { history: "push" });
  return {
    ...route,
    tab: route.tab ?? (route.trace ? "traces" : "home"),
    startChat: (bot: string | null) =>
      setRoute({
        tab: bot ? "bots" : "new_session",
        bot,
        chat: "new",
        report: null,
        annotations: null,
      }),
    navigate: (tab: WorkspaceTab) =>
      setRoute({
        tab,
        chat: null,
        report: null,
        annotations: null,
        trace: null,
      }),
    openChat: (chat: string, bot: string | null) =>
      setRoute({
        tab: bot ? "bots" : "home",
        chat,
        bot,
        report: null,
        annotations: null,
      }),
    openBot: (bot: string) =>
      setRoute({
        tab: "bots",
        bot,
        chat: null,
        report: null,
        annotations: null,
      }),
    openReport: (report: string | null) =>
      setRoute({ report, annotations: null }),
    openAnnotations: (annotations: AnnotationSelection | null) =>
      setRoute({ annotations, report: null }),
  };
}
