"use client";

import { useQuery } from "@tanstack/react-query";
import { ScanLine } from "lucide-react";
import { Inspector, useInspector } from "../../shared/Inspector";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../ui/select";
import { useNow } from "../../../hooks/useNow";
import { formatActivityTimestamp } from "../../../utils/activityTimestamp";
import { useLensApi } from "../data/LensServices";
import { useLensUpdate } from "../data/mutations";
import { lensQueries } from "../data/queries";
import { agoLabel } from "../model/format";
import { findingFrequency, percentLabel } from "../model/frequency";
import {
  ALL_AGENTS,
  filterInbox,
  findingKey,
  inboxAgents,
  inboxFinding,
  inboxRows,
  inboxSampledRuns,
  type InboxRow,
  type Priority,
} from "../model/inbox";
import type { Finding } from "../model/types";
import { useEvidenceRoute, useInboxFilters, useIssueRoute } from "../route";
import { FINDING_PANEL_WIDTH_KEY } from "../storage";
import { LensPageHeader } from "../ui/LensPageHeader";
import { useWorkerConnected } from "../hooks/useWorkerConnected";
import { AutomaticAnalysis } from "./AutomaticAnalysis";
import { EvidenceView } from "./Evidence";
import { FindingDetails } from "./FindingDetails";
import { LoadingState } from "../../shared/LoadingState";
import { InvestigationError } from "./InvestigationStates";
import { PRIORITY_LABEL, PRIORITY_ORDER, PriorityDot } from "./PriorityMark";

const PRIORITIES: { value: Priority | "all"; label: string }[] = [
  { value: "all", label: "All priorities" },
  { value: "high", label: "High" },
  { value: "medium", label: "Medium" },
  { value: "low", label: "Low" },
];

function FilterSelect<T extends string>({
  label,
  value,
  items,
  onChange,
}: {
  label: string;
  value: T;
  items: { value: T; label: string }[];
  onChange: (value: T) => void;
}) {
  return (
    <Select items={items} value={value} onValueChange={(next: T | null) => next !== null && onChange(next)}>
      <SelectTrigger
        size="sm"
        className="h-8 min-w-0 flex-1 bg-background font-mono text-[11px] shadow-none hover:bg-muted"
        aria-label={label}
      >
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {items.map((item) => (
          <SelectItem key={item.value} value={item.value}>
            {item.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

function FindingRow({ row, now }: { row: InboxRow; now: number }) {
  const frequency = findingFrequency(inboxFinding(row).occurrences, inboxSampledRuns(row));
  const percent = percentLabel(frequency.affected, frequency.total);
  return (
    <Inspector.Row
      item={row}
      render={
        <div
          role="row"
          tabIndex={0}
          aria-label={row.title}
          className="mx-2 block cursor-pointer space-y-2 rounded-md border border-transparent px-3 py-3 transition-colors duration-150 outline-none hover:border-border hover:bg-muted/40 focus-visible:ring-2 focus-visible:ring-ring/50 data-[state=selected]:border-border data-[state=selected]:bg-trace-row-selected"
        />
      }
    >
      <div role="gridcell" className="line-clamp-2 text-[13px] leading-5 font-medium text-pretty text-foreground">
        {row.title}
      </div>
      <div className="flex flex-wrap items-center justify-between gap-x-2 gap-y-1 font-mono text-[10px] text-muted-foreground">
        <span className="tabular-nums" title={formatActivityTimestamp(row.lastSeen)}>
          {agoLabel(Date.parse(row.lastSeen), now)}
        </span>
        <span
          className="whitespace-nowrap text-foreground/75 tabular-nums"
          title={`${row.runs} affected ${row.runs === 1 ? "trace" : "traces"} across ${row.sources.map(({ lens }) => lens.settings.name).join(", ")}`}
        >
          {percent ? `${percent} affected` : `${row.runs} ${row.runs === 1 ? "trace" : "traces"}`}
        </span>
      </div>
    </Inspector.Row>
  );
}

function FindingList({ rows, now }: { rows: readonly InboxRow[]; now: number }) {
  const groups = PRIORITY_ORDER.map((priority) => ({
    priority,
    rows: rows.filter((row) => row.priority === priority),
  })).filter((group) => group.rows.length > 0);
  return (
    <div role="grid" aria-label="Findings" className="min-h-0 flex-1 overflow-y-auto overscroll-contain pb-2">
      {groups.map((group) => (
        <div role="rowgroup" key={group.priority} aria-label={`${PRIORITY_LABEL[group.priority]} priority findings`}>
          <div
            role="row"
            className="sticky top-0 z-raised flex items-center gap-2 border-b bg-background px-4 py-3 font-mono text-[10px] font-medium tracking-wide text-muted-foreground"
          >
            <PriorityDot priority={group.priority} />
            <span role="columnheader">{PRIORITY_LABEL[group.priority]} priority</span>
            <span className="ml-auto rounded border bg-muted/30 px-1.5 tabular-nums">{group.rows.length}</span>
          </div>
          <div className="space-y-1 py-2">
            {group.rows.map((row) => (
              <FindingRow key={row.key} row={row} now={now} />
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}

function InboxDetail({
  row,
  readOnly,
  busy,
  onReview,
}: {
  row: InboxRow;
  readOnly: boolean;
  busy: boolean;
  onReview: (row: InboxRow, status: Finding["status"], reason: string) => void;
}) {
  const { close } = useInspector<InboxRow>();
  const { evidence, setEvidence } = useEvidenceRoute();
  const owner =
    row.sources.find(({ finding }) => finding.evidence.some((quote) => quote.execution_id === evidence?.id)) ??
    row.sources[0];
  return (
    <>
      <div hidden={evidence !== null} className={evidence ? undefined : "flex min-h-0 flex-1 flex-col"}>
        <FindingDetails
          key={row.key}
          finding={inboxFinding(row)}
          agents={row.agents}
          sampledRuns={inboxSampledRuns(row)}
          readOnly={readOnly}
          busy={busy}
          onOpenEvidence={setEvidence}
          onReview={(status, reason) => onReview(row, status, reason)}
          onClose={close}
        />
      </div>
      {evidence && (
        <EvidenceView
          lensId={owner.lens.id}
          evidence={evidence}
          backLabel="Back to finding"
          onBack={() => setEvidence(null)}
        />
      )}
    </>
  );
}

export function FindingsView({ readOnly = false }: { readOnly?: boolean }) {
  const api = useLensApi();
  const list = useQuery(lensQueries.list(api));
  const update = useLensUpdate();
  const { issueKey, setIssueKey } = useIssueRoute();
  const filters = useInboxFilters();
  const now = useNow(30000);
  const all = inboxRows(list.data?.lenses ?? []);
  const rows = filterInbox(all, filters);
  const selected =
    all.find((row) => row.sources.some(({ lens, finding }) => findingKey(lens, finding) === issueKey)) ?? null;
  const agents = [
    { value: ALL_AGENTS, label: "All agents" },
    ...inboxAgents(all).map((agent) => ({ value: agent, label: agent })),
  ];
  const review = (row: InboxRow, status: Finding["status"], reason: string) => {
    update.mutate(
      async (api) => {
        const results = await Promise.allSettled(
          row.sources.map(({ lens, finding }) => api.reviewFinding(lens.id, finding.id, status, reason)),
        );
        const failed = results.find((result) => result.status === "rejected");
        if (failed) throw failed.reason;
      },
      { onSuccess: () => setIssueKey(null) },
    );
  };
  const connected = useWorkerConnected(list.data?.workers);
  if (list.isPending)
    return <LoadingState title="Loading findings…" description="Fetching findings and analysis status." />;
  return (
    <Inspector.Root
      items={rows}
      itemKey={(row) => row.key}
      selected={selected}
      onSelectedChange={(row) => setIssueKey(row?.key ?? null)}
      noun="finding"
      storageKey={FINDING_PANEL_WIDTH_KEY}
    >
      <div className="@container/findings flex min-h-0 flex-1 flex-col overflow-hidden bg-background">
        <LensPageHeader title="Findings" description="Review what your agents need to improve." />
        <AutomaticAnalysis lenses={list.data?.lenses ?? []} readOnly={readOnly} ready={connected} />
        {(list.error || update.error) && (
          <InvestigationError
            message={(list.error ?? update.error)!.message}
            refresh={() => {
              update.reset();
              void list.refetch();
            }}
          />
        )}
        <div className="lens-panel flex min-h-0 flex-1 overflow-hidden rounded-lg border">
          <section
            aria-label="Findings list"
            className={`flex min-h-0 w-full flex-col border-r @3xl/findings:w-72 @3xl/findings:shrink-0 @5xl/findings:w-80 ${selected ? "hidden @3xl/findings:flex" : "flex"}`}
          >
            <div className="lens-toolbar flex shrink-0 items-center gap-2 border-b px-3 py-3">
              <FilterSelect label="Filter by agent" value={filters.agent} items={agents} onChange={filters.setAgent} />
              <FilterSelect
                label="Filter by priority"
                value={filters.priority}
                items={PRIORITIES}
                onChange={filters.setPriority}
              />
            </div>
            <FindingList rows={rows} now={now} />
            {!rows.length && !list.error && (
              <p className="lens-empty-state px-4 py-16 text-center text-xs leading-6 text-muted-foreground">
                {all.length
                  ? "No findings match these filters."
                  : "No findings yet. Automatic analysis checks your traces and reports problems here."}
              </p>
            )}
            <footer className="flex h-9 shrink-0 items-center border-t px-3 font-mono text-[10px] text-muted-foreground">
              {rows.length} {rows.length === 1 ? "finding" : "findings"}
              {rows.length !== all.length && ` of ${all.length}`}
            </footer>
          </section>
          {selected ? (
            <aside
              aria-label="Finding details"
              data-testid="finding-panel"
              className="flex min-h-0 min-w-0 flex-1 flex-col"
            >
              <InboxDetail row={selected} readOnly={readOnly} busy={update.isPending} onReview={review} />
            </aside>
          ) : (
            <div className="lens-empty-state hidden min-w-0 flex-1 flex-col items-center justify-center gap-5 p-8 text-sm text-muted-foreground @3xl/findings:flex">
              <span className="flex size-14 items-center justify-center rounded-xl border border-indigo-200 bg-background text-indigo-600 dark:border-indigo-400/30 dark:text-indigo-300">
                <ScanLine aria-hidden="true" className="size-6" strokeWidth={1.5} />
              </span>
              {rows.length ? (
                <p className="max-w-64 text-center text-xs leading-6">
                  Select a finding to see how often it happens and where.
                </p>
              ) : null}
            </div>
          )}
        </div>
      </div>
    </Inspector.Root>
  );
}
