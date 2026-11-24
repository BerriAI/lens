"use client";

import {
  parseAsBoolean,
  parseAsString,
  parseAsStringLiteral,
  useQueryStates,
} from "nuqs";

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
      }),
    navigate: (tab: WorkspaceTab) =>
      setRoute({ tab, chat: null, report: null, trace: null }),
    openChat: (chat: string, bot: string | null) =>
      setRoute({ tab: bot ? "bots" : "home", chat, bot, report: null }),
    openBot: (bot: string) =>
      setRoute({ tab: "bots", bot, chat: null, report: null }),
    openReport: (report: string | null) => setRoute({ report }),
  };
}
