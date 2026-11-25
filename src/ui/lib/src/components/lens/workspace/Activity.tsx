import { useContext, useRef } from "react";
import { useInfiniteQuery, useMutation } from "@tanstack/react-query";
import { Copy, MessageSquare, Tag } from "lucide-react";
import type {
  AnnotationActivity,
  AnnotationSelection,
  TraceAnnotation,
} from "../../../lib/http/agentWorkspace";
import { Button } from "../../ui/button";
import { Sheet, SheetContent, SheetTitle } from "../../ui/sheet";
import { copyToClipboard } from "../../../utils/dataUtils";
import { useOpenSourceTrace } from "../route";
import { WorkspaceScope } from "./Results";
import type { WorkspaceApi } from "./Chat";

export function ActivityPost({
  activity,
  onOpen,
}: {
  activity: AnnotationActivity;
  onOpen: (selection: AnnotationSelection) => void;
}) {
  return (
    <button
      type="button"
      onClick={() =>
        onOpen({
          producer_id: activity.producer_id,
          definition_id: activity.definition_id,
          value: activity.value,
        })
      }
      title={`Current total for ${activity.definition_key}: ${String(activity.value)}. Last updated ${new Date(activity.updated_at).toLocaleString()}`}
      className="flex w-full items-center gap-2 px-1 py-1 text-left text-xs text-muted-foreground hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
    >
      <Tag aria-hidden="true" className="size-3 shrink-0" />
      <span className="min-w-0 truncate">
        <span className="font-medium">{activity.producer_name}</span> tagged{" "}
        {activity.count.toLocaleString()}{" "}
        {activity.count === 1 ? "trace" : "traces"}
        <span aria-hidden="true"> · </span>
        <span className="text-foreground">{String(activity.value)}</span>
      </span>
      <time
        className="ml-auto shrink-0 text-[11px]"
        dateTime={new Date(activity.updated_at).toISOString()}
      >
        {new Date(activity.updated_at).toLocaleTimeString(undefined, {
          hour: "numeric",
          minute: "2-digit",
        })}
      </time>
    </button>
  );
}

export function ActivityPanel({
  api,
  selection,
  onClose,
  writable,
  onChat,
}: {
  api: WorkspaceApi;
  selection: AnnotationSelection | null;
  onClose: () => void;
  writable: boolean;
  onChat: (id: string) => void;
}) {
  const scope = useContext(WorkspaceScope);
  const openTrace = useOpenSourceTrace();
  const annotations = useInfiniteQuery({
    queryKey: ["agent-workspace", scope, "annotations", selection],
    queryFn: ({ pageParam }) => api.queryAnnotations(selection!, pageParam),
    initialPageParam: undefined as number | undefined,
    getNextPageParam: (page) => page.next_cursor ?? undefined,
    enabled: !!selection,
  });
  const rows =
    annotations.data?.pages.flatMap((page) => page.annotations) ?? [];
  const first = rows[0];
  const copy = useMutation({
    mutationFn: async () => {
      if (!selection) return;
      const all: TraceAnnotation[] = [];
      let after: number | undefined;
      do {
        const page = await api.queryAnnotations(selection, after);
        all.push(...page.annotations);
        after = page.next_cursor ?? undefined;
      } while (after !== undefined);
      await copyToClipboard(
        JSON.stringify(
          {
            lens_tool: { name: "query_annotations", arguments: selection },
            annotations: all,
          },
          null,
          2,
        ),
        "Annotations and trace references copied",
      );
    },
  });
  const pending = useRef<{ id: string; chat: string | null } | null>(null);
  const discuss = useMutation({
    mutationFn: async () => {
      pending.current ??= { id: crypto.randomUUID(), chat: null };
      const request = pending.current;
      request.chat ??= (
        await api.createConversation(
          "Investigate tagged traces",
          undefined,
          request.id,
        )
      ).id;
      await api.send(
        request.chat,
        request.id,
        `Investigate the traces matching these annotation filters: ${JSON.stringify(selection)}. Use query_annotations, follow pagination, then read the relevant trace evidence. Explain what you find.`,
      );
      return request.chat;
    },
    onSuccess: (id) => {
      pending.current = null;
      onClose();
      onChat(id);
    },
  });
  return (
    <Sheet
      open={!!selection}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <SheetContent
        className="lens-shell-overlay w-full! gap-0 overflow-y-auto p-0 sm:max-w-2xl!"
        aria-describedby={undefined}
      >
        <header className="border-b px-6 py-5 pr-12">
          <SheetTitle>
            {first
              ? `${first.definition_key}: ${String(first.value)}`
              : "Tagged traces"}
          </SheetTitle>
          {first && (
            <p className="mt-2 text-xs text-muted-foreground">
              Tagged by {first.producer_name}
            </p>
          )}
        </header>
        <div className="p-6">
          <div className="mb-5 flex gap-2">
            <Button
              variant="outline"
              size="sm"
              disabled={!writable || discuss.isPending || !first}
              onClick={() => discuss.mutate()}
            >
              <MessageSquare className="size-3.5" />
              Discuss
            </Button>
            <Button
              variant="ghost"
              size="sm"
              disabled={copy.isPending || !first}
              onClick={() => copy.mutate()}
            >
              <Copy className="size-3.5" />
              Copy for agent
            </Button>
          </div>
          {(annotations.error || copy.error || discuss.error) && (
            <p role="alert" className="text-sm text-destructive">
              {annotations.error?.message ??
                copy.error?.message ??
                discuss.error?.message}
            </p>
          )}
          {annotations.isLoading && (
            <p role="status" className="text-sm text-muted-foreground">
              Loading tagged traces…
            </p>
          )}
          {!annotations.isLoading && !annotations.error && !rows.length && (
            <p className="text-sm text-muted-foreground">
              No traces currently match this tag
            </p>
          )}
          <div className="divide-y">
            {rows.map((annotation) => (
              <article
                key={`${annotation.trace.trace_id}:${annotation.trace.trace_ref}`}
                className="py-4 first:pt-0"
              >
                <button
                  type="button"
                  className="max-w-full truncate font-mono text-xs text-primary hover:underline"
                  onClick={() => {
                    onClose();
                    openTrace({
                      traceId: annotation.trace.trace_id,
                      traceRef: annotation.trace.trace_ref,
                      spanId: "",
                    });
                  }}
                >
                  {annotation.trace.trace_id}
                </button>
                <p className="mt-2 text-sm leading-relaxed whitespace-pre-wrap">
                  {annotation.evidence}
                </p>
              </article>
            ))}
          </div>
          {annotations.hasNextPage && (
            <Button
              size="sm"
              variant="outline"
              disabled={annotations.isFetchingNextPage}
              onClick={() => void annotations.fetchNextPage()}
            >
              Load more traces
            </Button>
          )}
        </div>
      </SheetContent>
    </Sheet>
  );
}
