import { useEffect, useRef, useState, type ReactNode } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowUp, Loader2, Square } from "lucide-react";
import type {
  ConversationState,
  WorkspaceAgent,
} from "../../../lib/http/agentWorkspace";
import { Button } from "../../ui/button";
import { Textarea } from "../../ui/textarea";
import { Markdown } from "../traces/detail/content/Markdown";
import { assistantText, toolActivity } from "./messages";
import type { workspaceApi } from "./api";

import { ModelPicker } from "./ModelPicker";

export type WorkspaceApi = ReturnType<typeof workspaceApi>;
export const conversationKey = (scope: string, id: string | null) =>
  ["agent-workspace", scope, "conversation", id] as const;

export function Chat({
  api,
  scope,
  chatId,
  bot,
  writable,
  onOpened,
  children,
  home = false,
  defaultModel,
}: {
  api: WorkspaceApi;
  scope: string;
  chatId: string | null;
  bot?: WorkspaceAgent;
  writable: boolean;
  onOpened: (id: string) => void;
  children?: ReactNode;
  home?: boolean;
  defaultModel: string;
}) {
  const client = useQueryClient();
  const [model, setModel] = useState<string>();
  const [draft, setDraft] = useState("");
  const pending = useRef<{
    id: string;
    text: string;
    conversation: string | null;
    model: string;
  } | null>(null);
  const bottom = useRef<HTMLDivElement>(null);
  const queryKey = conversationKey(scope, chatId);
  const state = useQuery({
    queryKey,
    queryFn: () =>
      api.conversation(
        chatId!,
        client.getQueryData<ConversationState>(queryKey),
      ),
    enabled: chatId !== null,
    refetchInterval: (query) =>
      query.state.data?.turns.some((turn) =>
        ["queued", "running"].includes(turn.status),
      )
        ? 800
        : bot?.enabled
          ? 5000
          : false,
  });
  const running =
    state.data?.turns.some((turn) =>
      ["queued", "running"].includes(turn.status),
    ) ?? false;
  const send = useMutation({
    mutationFn: async () => {
      const text = draft.trim();
      if (
        !pending.current ||
        pending.current.text !== text ||
        pending.current.model !== (model ?? defaultModel)
      )
        pending.current = {
          id: crypto.randomUUID(),
          text,
          conversation: chatId,
          model: model ?? defaultModel,
        };
      const message = pending.current;
      if (!message.conversation)
        message.conversation = (
          await api.createConversation(
            text.slice(0, 100),
            bot?.id,
            message.id,
            message.model,
          )
        ).id;
      await api.send(message.conversation, message.id, message.text);
      return message.conversation;
    },
    onSuccess: (id) => {
      pending.current = null;
      setDraft("");
      onOpened(id);
      void client.invalidateQueries({ queryKey: ["agent-workspace", scope] });
    },
  });
  const stop = useMutation({
    mutationFn: () => api.stop(chatId!),
    onSuccess: () => void state.refetch(),
  });
  const lastEvent = state.data?.events.at(-1)?.sequence;
  useEffect(() => {
    bottom.current?.scrollIntoView({ block: "nearest" });
  }, [lastEvent, state.data?.turns.length]);
  const disabled = !writable || bot?.enabled === false;
  const composer = (
    <form
      className={
        home && !chatId
          ? "mx-auto w-full max-w-3xl"
          : "mx-auto w-full max-w-3xl shrink-0 px-5 pt-2 pb-5 md:px-0"
      }
      onSubmit={(event) => {
        event.preventDefault();
        if (draft.trim() && !send.isPending && !disabled) send.mutate();
      }}
    >
      {(send.error || stop.error) && (
        <p role="alert" className="mb-2 text-sm text-destructive">
          {send.error?.message ?? stop.error?.message}
        </p>
      )}
      <div className="rounded-xl border bg-card p-3 shadow-xs focus-within:ring-1 focus-within:ring-ring">
        <Textarea
          aria-label={bot ? `Message ${bot.name}` : "Message Lens"}
          placeholder={
            disabled
              ? bot?.enabled === false
                ? "Resume this bot to continue"
                : "A Lens admin can start agent conversations"
              : "Ask anything about your traces…"
          }
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          disabled={disabled || send.isPending}
          rows={2}
          maxLength={100000}
          className="max-h-48 min-h-14 resize-y border-0 bg-transparent p-0 text-sm shadow-none focus-visible:ring-0 dark:bg-transparent"
          onKeyDown={(event) => {
            if (
              event.key === "Enter" &&
              !event.shiftKey &&
              !event.nativeEvent.isComposing
            ) {
              event.preventDefault();
              event.currentTarget.form?.requestSubmit();
            }
          }}
        />
        <div className="mt-2 flex items-center justify-end gap-2">
          <div className="mr-auto min-w-0">
            {!chatId && !bot ? (
              <ModelPicker
                api={api}
                value={model ?? defaultModel}
                onChange={setModel}
                disabled={disabled || send.isPending}
              />
            ) : (
              <span
                className="block truncate text-xs text-muted-foreground"
                title={
                  state.data?.conversation.model ?? bot?.model ?? defaultModel
                }
              >
                {state.data?.conversation.model ?? bot?.model ?? defaultModel}
              </span>
            )}
          </div>
          {running && (
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={stop.isPending || !writable}
              onClick={() => stop.mutate()}
            >
              <Square className="size-3" />
              Stop
            </Button>
          )}
          <Button
            type="submit"
            size="icon-sm"
            aria-label="Send message"
            disabled={!draft.trim() || disabled || send.isPending}
          >
            {send.isPending ? (
              <Loader2 className="size-4 animate-spin" />
            ) : (
              <ArrowUp className="size-4" />
            )}
          </Button>
        </div>
      </div>
    </form>
  );
  if (home && !chatId)
    return (
      <div className="flex min-h-0 flex-1 flex-col">
        <div className="shrink-0 bg-card px-5 pt-5 pb-4 md:px-8">
          {composer}
        </div>
        <div
          aria-label="Trace feed"
          className="min-h-0 flex-1 overflow-y-auto px-5 pt-2 pb-6 md:px-8"
        >
          <div className="mx-auto w-full max-w-3xl">{children}</div>
        </div>
      </div>
    );
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="min-h-0 flex-1 overflow-y-auto px-5 py-6 md:px-8">
        <div className="mx-auto w-full max-w-3xl">
          {!chatId && (
            <div className="pt-8 pb-7 md:pt-16">
              <h2 className="text-xl font-medium tracking-tight">
                {bot
                  ? `Chat with ${bot.name}`
                  : "What would you like to understand?"}
              </h2>
              <p className="mt-2 text-sm text-muted-foreground">
                {bot
                  ? "Start a separate conversation with this bot"
                  : "Explore your traces, investigate a problem, or give a bot an ongoing task"}
              </p>
            </div>
          )}
          {state.isLoading && (
            <p
              role="status"
              className="flex items-center gap-2 text-sm text-muted-foreground"
            >
              <Loader2 className="size-4 animate-spin" />
              Loading conversation…
            </p>
          )}
          {state.error && (
            <p role="alert" className="text-sm text-destructive">
              {state.error.message}{" "}
              <Button
                variant="link"
                size="sm"
                onClick={() => void state.refetch()}
              >
                Retry
              </Button>
            </p>
          )}
          <div role="log" aria-label="Conversation" className="space-y-8">
            {state.data?.native_history?.truncated && (
              <p className="text-xs text-muted-foreground">
                Showing the latest 500 native messages. The bot keeps its
                earlier context
              </p>
            )}
            {state.data?.native_history?.messages.map((message) =>
              message.role === "user" ? (
                <div
                  key={message.id}
                  className="ml-auto w-fit max-w-[90%] rounded-lg bg-muted px-4 py-3 text-sm whitespace-pre-wrap break-words"
                >
                  {message.content}
                </div>
              ) : (
                <section key={message.id} className="space-y-3">
                  <p className="text-xs font-medium text-muted-foreground">
                    {bot?.name ?? "Lens"}
                  </p>
                  <Markdown text={message.content} />
                </section>
              ),
            )}
            {state.data?.turns.map((turn) => {
              if (state.data.native_history && turn.status === "completed")
                return null;
              const events = state.data.events.filter(
                (event) => event.turn_id === turn.id,
              );
              const response = assistantText(events);
              const tools = toolActivity(events);
              return (
                <section key={turn.id} className="space-y-4">
                  <div className="ml-auto w-fit max-w-[90%] rounded-lg bg-muted px-4 py-3 text-sm whitespace-pre-wrap break-words">
                    {turn.input}
                  </div>
                  <div className="space-y-3">
                    <p className="text-xs font-medium text-muted-foreground">
                      {bot?.name ?? "Lens"}
                    </p>
                    {tools.length > 0 && (
                      <details className="text-xs text-muted-foreground">
                        <summary className="cursor-pointer py-1">
                          {tools.length} tool{" "}
                          {tools.length === 1 ? "call" : "calls"}
                        </summary>
                        <ul className="mt-2 space-y-1 border-l pl-3">
                          {tools.map((tool) => (
                            <li key={tool.id} className="flex gap-3">
                              <span className="min-w-0 break-words">
                                {tool.title}
                              </span>
                              <span className="ml-auto shrink-0">
                                {tool.status}
                              </span>
                            </li>
                          ))}
                        </ul>
                      </details>
                    )}
                    {response && <Markdown text={response} />}
                    {["queued", "running"].includes(turn.status) && (
                      <p
                        role="status"
                        className="flex items-center gap-2 text-xs text-muted-foreground"
                      >
                        <Loader2 className="size-3 animate-spin" />
                        {turn.status === "queued" ? "Queued" : "Working…"}
                      </p>
                    )}
                    {["failed", "interrupted"].includes(turn.status) && (
                      <p role="alert" className="text-sm text-destructive">
                        {turn.error ??
                          "This turn stopped. You can continue the conversation"}
                      </p>
                    )}
                  </div>
                </section>
              );
            })}
          </div>
          {!chatId && children}
          <div ref={bottom} />
        </div>
      </div>
      {composer}
    </div>
  );
}
