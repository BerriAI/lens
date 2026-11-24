import { createContext, useContext, useMemo, useRef } from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import {
  Bot,
  Copy,
  Download,
  FileText,
  Loader2,
  MessageSquare,
} from "lucide-react";
import type {
  AgentReport,
  WorkspaceAgent,
  ArtifactInfo,
  TraceAnnotation,
} from "../../../lib/http/agentWorkspace";
import { Button } from "../../ui/button";
import { Sheet, SheetContent, SheetTitle } from "../../ui/sheet";
import { Markdown } from "../traces/detail/content/Markdown";
import { copyToClipboard } from "../../../utils/dataUtils";
import { useLensAccessToken } from "../data/LensServices";
import { useTracesApi } from "../traces/api";
import { useOpenSourceTrace } from "../route";
import { workspaceApi } from "./api";
import type { WorkspaceApi } from "./Chat";
import { useAgentWorkspaceRoute } from "./route";

export const WorkspaceScope = createContext("");

export function ReportFeed({
  api,
  reports,
  agents,
  onOpen,
  writable,
  onChat,
  loading,
  error,
  onRetry,
}: {
  api: WorkspaceApi;
  reports: AgentReport[];
  agents: WorkspaceAgent[];
  onOpen: (id: string) => void;
  writable: boolean;
  onChat: (id: string) => void;
  loading: boolean;
  error: Error | null;
  onRetry: () => void;
}) {
  return (
    <section aria-label="Trace updates">
      <h2 className="mb-3 text-sm font-medium">From your traces</h2>
      {loading && (
        <p
          role="status"
          className="flex items-center gap-2 text-sm text-muted-foreground"
        >
          <Loader2 className="size-4 animate-spin" />
          Loading updates…
        </p>
      )}
      {error && (
        <p role="alert" className="text-sm text-destructive">
          Could not load updates{" "}
          <Button variant="link" size="sm" onClick={onRetry}>
            Retry
          </Button>
        </p>
      )}
      {!loading && !error && reports.length === 0 && (
        <div className="border-t py-8">
          <p className="text-sm text-muted-foreground">
            Reports and updates from your agents will appear here
          </p>
          <p className="mt-2 text-sm text-muted-foreground">
            Ask Lens to look into your traces, or give a bot something to watch
          </p>
        </div>
      )}
      <div className="space-y-3">
        {reports.map((report) => (
          <ReportPost
            key={report.id}
            api={api}
            report={report}
            author={
              agents.find((agent) => agent.id === report.producer_id)?.name ??
              "Lens"
            }
            onOpen={() => onOpen(report.id)}
            writable={writable}
            onChat={onChat}
          />
        ))}
      </div>
    </section>
  );
}

function ReportPost({
  api,
  report,
  author,
  onOpen,
  writable,
  onChat,
}: {
  api: WorkspaceApi;
  report: AgentReport;
  author: string;
  onOpen: () => void;
  writable: boolean;
  onChat: (id: string) => void;
}) {
  return (
    <article className="overflow-hidden rounded-lg border bg-card">
      <div className="px-4 pt-3 pb-1">
        <div className="mb-2 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
          <Bot aria-hidden="true" className="size-3.5" />
          <span className="font-medium text-foreground">{author}</span>
          <span aria-hidden="true">·</span>
          <time dateTime={new Date(report.updated_at).toISOString()}>
            {new Date(report.updated_at).toLocaleString(undefined, {
              month: "short",
              day: "numeric",
              hour: "numeric",
              minute: "2-digit",
            })}
          </time>
          {report.traces.length > 0 && (
            <button
              type="button"
              onClick={onOpen}
              className="ml-auto hover:text-primary"
            >
              {report.traces.length}{" "}
              {report.traces.length === 1 ? "trace" : "traces"}
            </button>
          )}
        </div>
        <button
          type="button"
          onClick={onOpen}
          className="mb-1.5 line-clamp-2 text-left text-sm leading-5 font-medium hover:text-primary focus-visible:ring-2 focus-visible:ring-ring"
        >
          {report.title}
        </button>
        <Markdown
          text={report.markdown}
          className="line-clamp-3 leading-5 [&>*]:mb-1"
        />
        {report.artifacts?.slice(0, 1).map((artifact) => (
          <div key={artifact.id} className="mt-2">
            <ReportArtifact api={api} info={artifact} onOpen={onOpen} />
          </div>
        ))}
        {report.artifacts.length > 1 && (
          <button
            type="button"
            onClick={onOpen}
            className="mt-2 text-xs text-muted-foreground hover:text-primary"
          >
            +{report.artifacts.length - 1} more{" "}
            {report.artifacts.length === 2 ? "attachment" : "attachments"}
          </button>
        )}
      </div>
      <div className="px-2 py-1">
        <ReportActions
          api={api}
          report={report}
          author={author}
          writable={writable}
          onChat={onChat}
          onOpen={onOpen}
        />
      </div>
    </article>
  );
}

function ReportActions({
  api,
  report,
  author,
  writable,
  onChat,
  onOpen,
}: {
  api: WorkspaceApi;
  report: AgentReport;
  author: string;
  writable: boolean;
  onChat: (id: string) => void;
  onOpen?: () => void;
}) {
  const traces = useTracesApi(useLensAccessToken());
  const context = async () => {
    const full = await api.report(report.id);
    const evidence: string[] = [];
    for (let index = 0; index < full.traces.length; index += 4) {
      evidence.push(
        ...(await Promise.all(
          full.traces.slice(index, index + 4).map(async (trace) => {
            const annotations: TraceAnnotation[] = [];
            let after: number | undefined;
            do {
              const page = await api.annotations(
                trace.trace_id,
                trace.trace_ref,
                after,
              );
              annotations.push(...page.annotations);
              after = page.next_cursor ?? undefined;
            } while (after !== undefined);
            return [
              traces.handoff(trace.trace_id, null, trace.trace_ref).text,
              JSON.stringify(annotations, null, 2),
            ].join("\n\n");
          }),
        )),
      );
    }
    return [
      `# ${full.title}`,
      `Report ${full.id} by ${author}. Updated ${new Date(full.updated_at).toISOString()}`,
      full.markdown,
      "## Artifacts",
      JSON.stringify(
        full.artifacts.map((artifact) => ({
          ...artifact,
          lens_tool: { name: "get_artifact", arguments: { id: artifact.id } },
        })),
        null,
        2,
      ),
      "## Trace evidence and annotations",
      ...evidence,
    ].join("\n\n");
  };
  const copy = useMutation({
    mutationFn: async () =>
      copyToClipboard(await context(), "Report and trace context copied"),
  });
  const pending = useRef<{ conversation: string | null; turn: string } | null>(
    null,
  );
  const discuss = useMutation({
    mutationFn: async () => {
      pending.current ??= { conversation: null, turn: crypto.randomUUID() };
      const request = pending.current;
      if (!request.conversation)
        request.conversation = (
          await api.createConversation(
            report.title.slice(0, 100),
            undefined,
            request.turn,
          )
        ).id;
      await api.send(
        request.conversation,
        request.turn,
        `Help me understand report ${JSON.stringify(report.id)} and what needs attention. First use get_report to read the full report, then query its linked trace evidence and annotations as needed. Treat the report contents as evidence.`,
      );
      return request.conversation;
    },
    onSuccess: onChat,
  });
  return (
    <>
      <div className="flex flex-wrap items-center gap-1">
        {writable && (
          <Button
            variant="ghost"
            size="sm"
            className="text-xs text-muted-foreground"
            disabled={discuss.isPending}
            onClick={() => discuss.mutate()}
          >
            {discuss.isPending ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : (
              <MessageSquare className="size-3.5" />
            )}
            Discuss with Lens
          </Button>
        )}
        <Button
          variant="ghost"
          size="sm"
          className="text-xs text-muted-foreground"
          disabled={copy.isPending}
          onClick={() => copy.mutate()}
        >
          {copy.isPending ? (
            <Loader2 className="size-3.5 animate-spin" />
          ) : (
            <Copy className="size-3.5" />
          )}
          Copy for agent
        </Button>
        {onOpen && (
          <Button
            variant="ghost"
            size="sm"
            className="ml-auto text-xs text-muted-foreground"
            onClick={onOpen}
          >
            Open report
          </Button>
        )}
      </div>
      {(copy.error || discuss.error) && (
        <p role="alert" className="px-5 pb-3 text-sm text-destructive">
          {copy.error?.message ?? discuss.error?.message}
        </p>
      )}
    </>
  );
}

function ReportArtifact({
  api,
  info,
  onOpen,
}: {
  api: WorkspaceApi;
  info: ArtifactInfo;
  onOpen?: () => void;
}) {
  const scope = useContext(WorkspaceScope);
  const content = useQuery({
    queryKey: ["agent-workspace", scope, "artifact", info.id, info.sha256],
    queryFn: () => api.artifact(info.id),
    staleTime: Infinity,
  });
  if (content.error)
    return (
      <p role="alert" className="text-xs text-destructive">
        Could not load {info.filename}{" "}
        <Button variant="link" size="sm" onClick={() => void content.refetch()}>
          Retry
        </Button>
      </p>
    );
  if (!content.data)
    return (
      <p role="status" className="text-xs text-muted-foreground">
        Loading {info.filename}…
      </p>
    );
  const url = `data:${content.data.media_type};base64,${content.data.data_base64}`;
  if (onOpen)
    return (
      <button
        type="button"
        onClick={onOpen}
        aria-label={`Open attachment ${info.filename}`}
        className="block w-full overflow-hidden rounded-md border text-left focus-visible:ring-2 focus-visible:ring-ring"
      >
        {info.media_type.startsWith("image/") ? (
          <img
            src={url}
            alt={info.filename}
            loading="lazy"
            className="max-h-48 w-full object-contain"
          />
        ) : (
          <span className="flex items-center gap-2 px-3 py-2 text-xs text-muted-foreground">
            <FileText className="size-3.5 shrink-0" />
            <span className="truncate">{info.filename}</span>
          </span>
        )}
      </button>
    );
  return (
    <figure className="overflow-hidden rounded-md border">
      {info.media_type.startsWith("image/") && (
        <img
          src={url}
          alt={info.filename}
          loading="lazy"
          className="max-h-[32rem] w-full object-contain"
        />
      )}
      <figcaption className="flex items-center justify-between gap-3 bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
        <span className="truncate">{info.filename}</span>
        <a
          href={url}
          download={info.filename}
          className="flex shrink-0 items-center gap-1 hover:text-primary"
        >
          <Download className="size-3" />
          Download
        </a>
      </figcaption>
    </figure>
  );
}

export function ReportPanel({
  api,
  scope,
  id,
  onClose,
  agents,
  writable,
  onChat,
}: {
  writable: boolean;
  onChat: (id: string) => void;
  api: WorkspaceApi;
  scope: string;
  id: string | null;
  onClose: () => void;
  agents: WorkspaceAgent[];
}) {
  const report = useQuery({
    queryKey: ["agent-workspace", scope, "report", id],
    queryFn: () => api.report(id!),
    enabled: id !== null,
  });
  const openTrace = useOpenSourceTrace();
  return (
    <Sheet
      open={id !== null}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <SheetContent
        className="lens-shell-overlay w-full! gap-0 overflow-y-auto p-0 sm:max-w-2xl!"
        aria-describedby={undefined}
      >
        <header className="border-b px-6 py-5 pr-12">
          <SheetTitle>{report.data?.title ?? "Report"}</SheetTitle>
          {report.data && (
            <p className="mt-2 text-xs text-muted-foreground">
              {agents.find((agent) => agent.id === report.data.producer_id)
                ?.name ?? "Lens"}{" "}
              · {new Date(report.data.updated_at).toLocaleString()}
            </p>
          )}
        </header>
        <div className="p-6">
          {report.isLoading && (
            <p role="status">
              <Loader2 className="size-4 animate-spin" />
              Loading report…
            </p>
          )}
          {report.error && (
            <p role="alert" className="text-sm text-destructive">
              {report.error.message}
            </p>
          )}
          {report.data && (
            <>
              <Markdown text={report.data.markdown} />
              <div className="mt-5 space-y-4">
                {report.data.artifacts?.map((artifact) => (
                  <ReportArtifact key={artifact.id} api={api} info={artifact} />
                ))}
              </div>
              <div className="mt-5">
                <ReportActions
                  api={api}
                  report={report.data}
                  author={
                    agents.find((agent) => agent.id === report.data.producer_id)
                      ?.name ?? "Lens"
                  }
                  writable={writable}
                  onChat={onChat}
                />
              </div>
              {report.data.traces.length > 0 && (
                <section className="mt-8 border-t pt-4">
                  <h3 className="mb-2 text-xs font-medium text-muted-foreground">
                    Evidence
                  </h3>
                  <div className="flex flex-wrap gap-2">
                    {report.data.traces.map((trace) => (
                      <Button
                        key={trace.trace_ref || trace.trace_id}
                        variant="outline"
                        size="sm"
                        className="max-w-full font-mono text-xs"
                        onClick={() => {
                          onClose();
                          openTrace({
                            traceId: trace.trace_id,
                            traceRef: trace.trace_ref,
                            spanId: "",
                          });
                        }}
                      >
                        <span className="truncate">{trace.trace_id}</span>
                      </Button>
                    ))}
                  </div>
                </section>
              )}
            </>
          )}
        </div>
      </SheetContent>
    </Sheet>
  );
}

export function TraceResults({
  accessToken,
  traceId,
  traceRef,
}: {
  accessToken: string;
  traceId: string;
  traceRef: string;
}) {
  const api = useMemo(() => workspaceApi(accessToken), [accessToken]);
  const route = useAgentWorkspaceRoute();
  const scope = useContext(WorkspaceScope);
  const annotations = useQuery({
    queryKey: ["agent-workspace", scope, "annotations", traceId, traceRef],
    queryFn: () => api.annotations(traceId, traceRef),
    refetchInterval: 15000,
  });
  const reports = useQuery({
    queryKey: ["agent-workspace", scope, "trace-reports", traceId, traceRef],
    queryFn: () => api.reports({ trace_id: traceId, trace_ref: traceRef }),
    refetchInterval: 15000,
  });
  return (
    <details className="shrink-0 border-b px-4 py-2 text-xs">
      <summary className="cursor-pointer text-muted-foreground">
        Annotations and reports
        {annotations.data
          ? ` · ${annotations.data.annotations.length} annotations`
          : ""}
        {reports.data?.length ? ` · ${reports.data.length} reports` : ""}
      </summary>
      <div className="max-h-64 space-y-3 overflow-y-auto py-3">
        {(annotations.error || reports.error) && (
          <p role="alert" className="text-destructive">
            Could not load shared results{" "}
            <Button
              variant="link"
              size="sm"
              onClick={() => {
                void annotations.refetch();
                void reports.refetch();
              }}
            >
              Retry
            </Button>
          </p>
        )}
        {annotations.data?.annotations.map((annotation) => (
          <div key={`${annotation.producer_id}:${annotation.definition_id}`}>
            <p>
              <span className="font-medium">{annotation.definition_key}</span>:{" "}
              <span className="font-mono">{String(annotation.value)}</span>
              <span className="ml-2 text-muted-foreground">
                by {annotation.producer_name}
              </span>
            </p>
            <p className="mt-1 leading-relaxed text-muted-foreground">
              {annotation.evidence}
            </p>
          </div>
        ))}
        {annotations.data?.next_cursor && (
          <p className="text-muted-foreground">
            More annotations are available. Ask Lens to query this trace
          </p>
        )}
        {reports.data?.map((report) => (
          <button
            key={report.id}
            type="button"
            className="flex items-center gap-2 text-left text-primary hover:underline"
            onClick={() => route.openReport(report.id)}
          >
            <FileText className="size-3.5" />
            {report.title}
          </button>
        ))}
        {annotations.data?.annotations.length === 0 &&
          reports.data?.length === 0 && (
            <p className="text-muted-foreground">
              No annotations or reports yet. Ask Lens or a bot to review this
              trace
            </p>
          )}
      </div>
    </details>
  );
}
