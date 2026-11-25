"use client";

import { useMemo, useState } from "react";
import {
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { Loader2, Pause, Play } from "lucide-react";
import { useLensHost } from "../../host/LensHost";
import { cn } from "../../lib/cva.config";
import { Button } from "../ui/button";
import { LensServicesProvider, useLiveLensServices } from "./data/LensServices";
import { createLensDemo } from "./data/demo/createLensDemo";
import { browserDemoStorage } from "./data/demo/storage";
import { LensSidebar } from "./LensSidebar";
import { isProxyAdminRole } from "../../utils/roles";
import AgentTracesPage from "./traces/list/AgentTracesPage";
import { LensPageActionsContext } from "./ui/LensPageHeader";
import { workspaceApi } from "./workspace/api";
import { useAgentWorkspaceRoute } from "./workspace/route";
import { WorkspaceNavigation } from "./workspace/Navigation";
import { Chat } from "./workspace/Chat";
import { SlackConnection } from "./workspace/Slack";
import { Bots, CreateBot } from "./workspace/Bots";
import { ReportFeed, ReportPanel, WorkspaceScope } from "./workspace/Results";

import { ActivityPanel } from "./workspace/Activity";

type WorkspaceProps = {
  accessToken: string;
  userRole: string;
  readOnly: boolean;
  cacheScope?: string;
};

export function LensWorkspace(props: WorkspaceProps) {
  const route = useAgentWorkspaceRoute();
  const live = useLiveLensServices(props.accessToken);
  const [sample] = useState(() =>
    route.demo ? createLensDemo(Date.now(), browserDemoStorage()) : null,
  );
  return (
    <LensServicesProvider services={route.demo && sample ? sample : live}>
      <WorkspaceContent {...props} />
    </LensServicesProvider>
  );
}

function WorkspaceContent({
  accessToken,
  userRole,
  readOnly,
  cacheScope,
}: WorkspaceProps) {
  const route = useAgentWorkspaceRoute();
  const host = useLensHost();
  const embedded = host.surface === "embedded";
  const client = useQueryClient();
  const api = useMemo(() => workspaceApi(accessToken), [accessToken]);
  const [pageActions, setPageActions] = useState<HTMLDivElement | null>(null);
  const [creatingBot, setCreatingBot] = useState(false);
  const scope = cacheScope ?? accessToken;
  const baseKey = ["agent-workspace", scope];
  const state = useQuery({
    queryKey: [...baseKey, "state"],
    queryFn: api.state,
    enabled: !route.demo,
    refetchInterval: 10000,
  });
  const agents = state.data?.agents ?? [];
  const bots = agents.filter((agent) => agent.runtime === "hermes");
  const bot =
    route.tab === "bots"
      ? bots.find((agent) => agent.id === route.bot)
      : undefined;
  const actor = bot ?? agents.find((agent) => agent.runtime === "deepagents");
  const writable = !readOnly && !route.demo && state.data?.writable === true;
  const chats = useQuery({
    queryKey: [...baseKey, "chats", actor?.id],
    queryFn: () => api.conversations(actor!.id),
    enabled: !!actor && !route.demo,
    refetchInterval: 5000,
  });
  const selectedChat =
    route.chat === "new"
      ? null
      : (route.chat ??
        (bot
          ? (chats.data?.find((chat) => chat.permanent)?.id ?? null)
          : null));
  const reports = useInfiniteQuery({
    queryKey: [...baseKey, "reports"],
    queryFn: ({ pageParam }) => api.reports({ before: pageParam }),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (page) =>
      page.length === 50
        ? `${page[page.length - 1].updated_at}:${page[page.length - 1].id}`
        : undefined,
    enabled: !route.demo,
    refetchInterval: 15000,
  });
  const refresh = () => void client.invalidateQueries({ queryKey: baseKey });
  const enabled = useMutation({
    mutationFn: () => api.setEnabled(bot!.id, !bot!.enabled),
    onSuccess: refresh,
  });
  const navigate = (tab: typeof route.tab) => {
    void route.navigate(tab);
    host.navigation?.onNavigate();
  };
  return (
    <main
      className={cn(
        "lens-workspace lens-shell relative flex h-full min-h-0 w-full min-w-0 flex-1 bg-background",
        embedded && "lens-embedded",
      )}
    >
      <WorkspaceScope.Provider value={scope}>
        <LensPageActionsContext.Provider
          value={{ target: pageActions, tab: route.tab }}
        >
          <div className="@container/lens-frame flex min-h-0 min-w-0 flex-1">
            <LensSidebar onNavigate={navigate}>
              {(collapsed, close) => (
                <WorkspaceNavigation
                  tab={route.tab}
                  collapsed={collapsed}
                  bots={bots}
                  selectedBot={bot?.id ?? null}
                  conversations={chats.data ?? []}
                  selectedChat={selectedChat}
                  writable={writable}
                  onCreateBot={() => setCreatingBot(true)}
                  onNavigate={(tab) => {
                    navigate(tab);
                    close();
                  }}
                  onBot={(id) => {
                    void route.openBot(id);
                    close();
                  }}
                  onChat={(id) => {
                    void route.openChat(id, bot?.id ?? null);
                    close();
                  }}
                  onNewChat={() => {
                    void route.startChat(bot?.id ?? null);
                    close();
                  }}
                />
              )}
            </LensSidebar>
            <div
              className={cn(
                "flex min-h-0 min-w-0 flex-1 flex-col pt-12 md:pt-0",
                !embedded && "lens-workspace-paper",
              )}
            >
              {route.tab === "traces" && (
                <div
                  ref={setPageActions}
                  role="group"
                  aria-label="Trace actions"
                  className="flex shrink-0 flex-wrap items-center justify-end gap-2 px-5 py-3 empty:hidden"
                />
              )}
              {!bot &&
                selectedChat &&
                writable &&
                state.data?.slack_configured && (
                  <div className="flex shrink-0 justify-end px-5 py-2">
                    <SlackConnection
                      key={selectedChat}
                      api={api}
                      scope={scope}
                      chatId={selectedChat}
                    />
                  </div>
                )}
              {state.error && (
                <div
                  role="alert"
                  className="flex items-center gap-3 border-b px-5 py-3 text-sm text-destructive"
                >
                  {state.error.message}
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => void state.refetch()}
                  >
                    Retry
                  </Button>
                </div>
              )}
              {chats.error && (
                <p role="alert" className="px-5 py-3 text-sm text-destructive">
                  Could not load chats{" "}
                  <Button variant="link" onClick={() => void chats.refetch()}>
                    Retry
                  </Button>
                </p>
              )}
              {enabled.error && (
                <p role="alert" className="px-5 py-3 text-sm text-destructive">
                  {enabled.error.message}
                </p>
              )}
              <div
                className={cn(
                  "flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-card",
                  !embedded &&
                    route.tab !== "traces" &&
                    "m-2 rounded-xl border md:m-4",
                )}
              >
                {route.tab === "traces" ? (
                  <AgentTracesPage
                    accessToken={accessToken}
                    readOnly={readOnly}
                    canMintTracingKey={isProxyAdminRole(userRole)}
                    canViewFindings={false}
                    agentWorkspace={!route.demo}
                  />
                ) : route.tab === "bots" && !bot ? (
                  <Bots
                    bots={bots}
                    writable={writable}
                    onSelect={route.openBot}
                    onCreate={() => setCreatingBot(true)}
                  />
                ) : (
                  <>
                    {bot && (
                      <div className="shrink-0 border-b px-5 py-3">
                        <div className="flex items-center gap-3">
                          <h2 className="min-w-0 flex-1 truncate text-sm font-medium">
                            {bot.name}
                          </h2>
                          <span className="text-xs text-muted-foreground">
                            {bot.enabled ? "Active" : "Paused"}
                          </span>
                          {selectedChat &&
                            writable &&
                            state.data?.slack_configured && (
                              <SlackConnection
                                key={selectedChat}
                                api={api}
                                scope={scope}
                                chatId={selectedChat}
                              />
                            )}
                          {writable && (
                            <Button
                              variant="outline"
                              size="sm"
                              disabled={enabled.isPending}
                              onClick={() => enabled.mutate()}
                            >
                              {bot.enabled ? (
                                <Pause className="size-3.5" />
                              ) : (
                                <Play className="size-3.5" />
                              )}
                              {bot.enabled ? "Pause" : "Resume"}
                            </Button>
                          )}
                        </div>
                        <details className="mt-2 text-sm">
                          <summary className="cursor-pointer text-muted-foreground">
                            Standing task
                          </summary>
                          <p className="mt-3 max-w-3xl whitespace-pre-wrap leading-relaxed">
                            {bot.instructions}
                          </p>
                        </details>
                      </div>
                    )}
                    {state.isLoading || (bot && chats.isLoading) ? (
                      <p
                        role="status"
                        className="flex items-center gap-2 p-6 text-sm text-muted-foreground"
                      >
                        <Loader2 className="size-4 animate-spin" />
                        Loading workspace…
                      </p>
                    ) : (
                      <Chat
                        defaultModel={state.data?.model ?? ""}
                        key={selectedChat ?? `new-${bot?.id ?? "main"}`}
                        api={api}
                        scope={scope}
                        home={route.tab === "home"}
                        chatId={selectedChat}
                        bot={bot}
                        writable={writable}
                        onOpened={(id) =>
                          void route.openChat(id, bot?.id ?? null)
                        }
                      >
                        {route.tab === "home" && (
                          <ReportFeed
                            api={api}
                            reports={reports.data?.pages.flat() ?? []}
                            agents={agents}
                            onOpen={route.openReport}
                            writable={writable}
                            onChat={(id) => {
                              refresh();
                              void route.openChat(id, null);
                            }}
                            loading={reports.isLoading && !route.demo}
                            error={reports.error}
                            onRetry={() => void reports.refetch()}
                          />
                        )}
                        {route.tab === "home" && reports.hasNextPage && (
                          <Button
                            variant="outline"
                            size="sm"
                            className="mt-4"
                            disabled={reports.isFetchingNextPage}
                            onClick={() => void reports.fetchNextPage()}
                          >
                            Load earlier updates
                          </Button>
                        )}
                        {route.demo && (
                          <p className="mt-4 text-sm text-muted-foreground">
                            Sign in to chat with Lens and create bots
                          </p>
                        )}
                      </Chat>
                    )}
                  </>
                )}
              </div>
            </div>
          </div>
          <CreateBot
            defaultModel={state.data?.model ?? ""}
            api={api}
            open={creatingBot}
            onOpenChange={setCreatingBot}
            onCreated={(id) => {
              refresh();
              void route.openBot(id);
            }}
          />
          <ActivityPanel
            api={api}
            selection={route.annotations}
            onClose={() => void route.openAnnotations(null)}
            writable={writable}
            onChat={(id) => {
              refresh();
              void route.openChat(id, null);
            }}
          />
          <ReportPanel
            api={api}
            scope={scope}
            writable={writable}
            onChat={(id) => {
              refresh();
              void route.openChat(id, null);
            }}
            id={route.report}
            onClose={() => void route.openReport(null)}
            agents={agents}
          />
        </LensPageActionsContext.Provider>
      </WorkspaceScope.Provider>
    </main>
  );
}
