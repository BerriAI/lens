import type { ReactNode } from "react";
import {
  Activity,
  Bot,
  House,
  MessageSquare,
  Plus,
  SquarePen,
} from "lucide-react";
import { cn } from "../../../lib/cva.config";
import type {
  AgentConversation,
  WorkspaceAgent,
} from "../../../lib/http/agentWorkspace";
import { Button } from "../../ui/button";
import type { WorkspaceTab } from "./route";

export const navigation = [
  { tab: "home", title: "Home", icon: House },
  { tab: "new_session", title: "New Session", icon: SquarePen },
  { tab: "bots", title: "Bots", icon: Bot },
  { tab: "traces", title: "Traces", icon: Activity },
] as const;

export function WorkspaceNavigation({
  tab,
  collapsed,
  bots,
  selectedBot,
  conversations,
  selectedChat,
  onNavigate,
  onBot,
  onChat,
  onNewChat,
  onCreateBot,
  writable,
  footer,
}: {
  tab: WorkspaceTab;
  collapsed: boolean;
  bots: WorkspaceAgent[];
  selectedBot: string | null;
  conversations: AgentConversation[];
  selectedChat: string | null;
  onNavigate: (tab: WorkspaceTab) => void;
  onBot: (id: string) => void;
  onChat: (id: string) => void;
  onNewChat: () => void;
  onCreateBot: () => void;
  writable: boolean;
  footer?: ReactNode;
}) {
  return (
    <>
      <nav aria-label="Workspace" className="flex shrink-0 flex-col gap-1">
        {navigation.map(({ tab: item, title, icon: Icon }) => (
          <button
            key={item}
            type="button"
            onClick={() => onNavigate(item)}
            aria-current={tab === item ? "page" : undefined}
            title={collapsed ? title : undefined}
            data-active={tab === item ? "" : undefined}
            className={cn(
              "lens-nav-tab relative flex h-9 w-full min-w-0 shrink-0 items-center gap-2.5 rounded-md px-2.5 text-left text-[13px] font-medium text-muted-foreground outline-none hover:bg-sidebar-accent/60 focus-visible:ring-2 focus-visible:ring-ring data-active:bg-sidebar-accent data-active:text-sidebar-accent-foreground",
              collapsed && "justify-center px-0",
            )}
          >
            <Icon aria-hidden="true" className="size-4 shrink-0" />
            <span className={cn(collapsed && "sr-only")}>{title}</span>
          </button>
        ))}
      </nav>
      {!collapsed && tab !== "traces" && (
        <div className="mt-6 min-h-0 flex-1 overflow-y-auto">
          {tab === "bots" && (
            <section aria-label="Bots" className="mb-5">
              <div className="mb-1 flex items-center justify-between px-2.5">
                <h2 className="text-xs text-muted-foreground">Bots</h2>
                {writable && (
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label="Create bot"
                    onClick={onCreateBot}
                  >
                    <Plus className="size-3.5" />
                  </Button>
                )}
              </div>
              {bots.map((bot) => (
                <button
                  key={bot.id}
                  type="button"
                  aria-pressed={bot.id === selectedBot}
                  onClick={() => onBot(bot.id)}
                  className={cn(
                    "flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-left text-[13px] outline-none hover:bg-sidebar-accent focus-visible:ring-2 focus-visible:ring-ring",
                    bot.id === selectedBot && "bg-sidebar-accent font-medium",
                  )}
                >
                  <Bot
                    aria-hidden="true"
                    className="size-3.5 shrink-0 text-muted-foreground"
                  />
                  <span className="truncate">{bot.name}</span>
                  {!bot.enabled && (
                    <span className="ml-auto text-xs text-muted-foreground">
                      Paused
                    </span>
                  )}
                </button>
              ))}
              {bots.length === 0 && (
                <p className="px-2.5 text-xs text-muted-foreground">
                  No bots yet
                </p>
              )}
            </section>
          )}
          {(tab !== "bots" || selectedBot) && (
            <section aria-label="Chats">
              <div className="mb-1 flex items-center justify-between px-2.5">
                <h2 className="text-xs text-muted-foreground">
                  {tab === "bots" ? "Bot chats" : "Your chats"}
                </h2>
                {writable && (
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label="New chat"
                    onClick={onNewChat}
                  >
                    <Plus className="size-3.5" />
                  </Button>
                )}
              </div>
              {conversations.map((chat) => (
                <button
                  type="button"
                  key={chat.id}
                  onClick={() => onChat(chat.id)}
                  aria-pressed={chat.id === selectedChat}
                  className={cn(
                    "flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-left text-[13px] outline-none hover:bg-sidebar-accent focus-visible:ring-2 focus-visible:ring-ring",
                    chat.id === selectedChat && "bg-sidebar-accent font-medium",
                  )}
                >
                  <MessageSquare
                    aria-hidden="true"
                    className="size-3.5 shrink-0 text-muted-foreground"
                  />
                  <span className="truncate">{chat.title}</span>
                </button>
              ))}
              {conversations.length === 0 && (
                <p className="px-2.5 py-2 text-xs text-muted-foreground">
                  Your conversations appear here
                </p>
              )}
            </section>
          )}
        </div>
      )}
      {footer && <div className="mt-auto shrink-0 pt-4">{footer}</div>}
    </>
  );
}
