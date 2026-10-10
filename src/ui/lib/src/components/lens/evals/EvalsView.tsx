"use client";

import { useState } from "react";
import {
  getCoreRowModel,
  useReactTable,
  type ColumnDef,
  type TableOptions,
} from "@tanstack/react-table";
import {
  ChevronLeft,
  Code2,
  FlaskConical,
  Loader2,
  Plus,
  TriangleAlert,
} from "lucide-react";

import { Inspector } from "../../shared/Inspector";
import { InspectorTable } from "../../shared/InspectorTable";
import { StateMessage } from "../../shared/StateMessage";
import { Button } from "../../ui/button";
import { useDatasets } from "../datasets/api";
import type { DatasetSummary } from "../datasets/types";
import { useEvalRunRoute } from "../route";
import { FINDING_PANEL_WIDTH_KEY } from "../storage";
import { NewEval } from "./NewEval";
import { LensPageHeader } from "../ui/LensPageHeader";
import { useEvalDefinition, useEvals } from "./runs/api";
import { RunDetail } from "./runs/RunDetail";
import { RunsTab } from "./runs/RunsTab";
import type { EvalDefinition } from "./runs/types";
import { gateLabel, scorerLabel } from "./labels";

export function EvalsView() {
  const { evalName, runId, caseId, openEval, openRun, openCase } =
    useEvalRunRoute();
  const [creating, setCreating] = useState(false);
  if (runId)
    return (
      <div className="flex min-h-0 flex-1 flex-col overflow-y-auto bg-card">
        <RunDetail
          key={runId}
          runId={runId}
          caseId={caseId}
          onBack={() => openRun(null)}
          onOpenCase={openCase}
        />
      </div>
    );
  if (creating)
    return (
      <NewEval
        onCancel={() => setCreating(false)}
        onSaved={(name) => {
          setCreating(false);
          openEval(name);
        }}
      />
    );
  if (evalName)
    return (
      <EvalPage
        key={evalName}
        name={evalName}
        onBack={() => openEval(null)}
        onOpenRun={openRun}
      />
    );
  return (
    <>
      <LensPageHeader
        page="evals"
        title="Evals"
        description="Run your agent against saved test cases. Inspect every result and trace"
        actions={
          <Button size="sm" onClick={() => setCreating(true)}>
            <Plus /> New eval
          </Button>
        }
      />
      <div className="lens-page-body flex min-h-0 flex-1 flex-col">
        <EvalList onOpen={openEval} />
      </div>
    </>
  );
}

const loading = (title: string) => (
  <StateMessage
    role="status"
    icon={
      <Loader2 className="size-5 animate-spin motion-reduce:animate-none" />
    }
    title={title}
    description="This takes a moment."
  />
);

const failed = (title: string, error: Error) => (
  <StateMessage
    role="alert"
    tone="destructive"
    icon={<TriangleAlert className="size-5" />}
    title={title}
    description={error.message}
  />
);

const datasetLabel = (
  definition: EvalDefinition,
  datasets: readonly DatasetSummary[],
) => {
  const dataset = datasets.find(
    (item) => item.id === definition.spec.dataset_id,
  );
  const name = dataset?.name ?? definition.spec.dataset_id;
  return `${name}@${definition.spec.revision ?? dataset?.revision ?? "latest"}`;
};

function EvalList({ onOpen }: { onOpen: (name: string) => void }) {
  const evals = useEvals();
  const datasets = useDatasets();
  if (evals.isPending) return loading("Loading evals…");
  if (evals.error) return failed("Couldn't load evals", evals.error);
  if (evals.data.length === 0)
    return (
      <div className="lens-empty-state flex min-h-0 flex-1 flex-col items-center justify-center gap-3">
        <StateMessage
          role="status"
          icon={<FlaskConical className="size-5" />}
          title="No evals yet"
          description="Create an eval to choose test cases and scoring rules, then connect your agent code to run it."
        />
      </div>
    );
  return (
    <EvalTable
      evals={evals.data}
      datasets={datasets.data ?? []}
      onOpen={onOpen}
    />
  );
}

function EvalTable({
  evals,
  datasets,
  onOpen,
}: {
  evals: readonly EvalDefinition[];
  datasets: readonly DatasetSummary[];
  onOpen: (name: string) => void;
}) {
  const columns: ColumnDef<EvalDefinition>[] = [
    {
      id: "name",
      size: 280,
      header: "Eval",
      cell: ({ row }) => (
        <span className="flex min-w-0 items-center gap-2.5 font-mono">
          <FlaskConical
            aria-hidden="true"
            className="size-4 shrink-0 text-muted-foreground"
          />
          <span className="truncate font-medium" title={row.original.name}>
            {row.original.name}
          </span>
        </span>
      ),
    },
    {
      id: "execution",
      size: 150,
      header: "Execution",
      cell: ({ row }) => (
        <span className="inline-flex items-center gap-1.5 whitespace-nowrap text-muted-foreground">
          <Code2 aria-hidden="true" className="size-3.5" />
          {row.original.spec.agent_io ? "HTTP contract" : "Code / SDK"}
        </span>
      ),
    },
    {
      id: "agent",
      size: 160,
      header: "Agent",
      cell: ({ row }) => (
        <span className="block truncate" title={row.original.spec.agent}>
          {row.original.spec.agent}
        </span>
      ),
    },
    {
      id: "dataset",
      size: 280,
      header: "Dataset",
      cell: ({ row }) => (
        <span
          className="block truncate text-muted-foreground"
          title={datasetLabel(row.original, datasets)}
        >
          {datasetLabel(row.original, datasets)}
        </span>
      ),
    },
  ];
  const tableOptions: TableOptions<EvalDefinition> = {
    data: [...evals],
    columns,
    defaultColumn: { size: undefined },
    getRowId: (definition) => definition.name,
    autoResetAll: false,
    getCoreRowModel: getCoreRowModel(),
  };
  const table = useReactTable(tableOptions);
  return (
    <Inspector.Root
      items={evals}
      itemKey={(definition) => definition.name}
      selected={null}
      onSelectedChange={(definition) => definition && onOpen(definition.name)}
      noun="eval"
      storageKey={FINDING_PANEL_WIDTH_KEY}
    >
      <div className="lens-panel flex min-h-0 flex-1 flex-col overflow-hidden">
        <div className="lens-toolbar flex h-9 shrink-0 items-center gap-2 border-b px-3">
          <span className="text-xs text-muted-foreground">
            {evals.length} {evals.length === 1 ? "eval" : "evals"}
          </span>
        </div>
        <InspectorTable.Root table={table}>
          <InspectorTable.Grid
            aria-label="Evals"
            className="table-fixed text-sm"
            style={{ minWidth: 720 }}
          >
            <InspectorTable.Header />
            <InspectorTable.Body<EvalDefinition> rowHeight={() => 52}>
              {(row) => (
                <InspectorTable.Row
                  row={row}
                  item={row.original}
                  tabIndex={0}
                  aria-label={row.original.name}
                  className="h-[52px]"
                />
              )}
            </InspectorTable.Body>
          </InspectorTable.Grid>
        </InspectorTable.Root>
      </div>
    </Inspector.Root>
  );
}

function EvalPage({
  name,
  onBack,
  onOpenRun,
}: {
  name: string;
  onBack: () => void;
  onOpenRun: (runId: string) => void;
}) {
  const definition = useEvalDefinition(name);
  const datasets = useDatasets();
  if (definition.isPending) return loading("Loading eval…");
  if (definition.error) return failed("Couldn't load eval", definition.error);
  const spec = definition.data.spec;
  const dataset = datasets.data?.find((item) => item.id === spec.dataset_id);
  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden bg-card">
      <LensPageHeader
        title={name}
        navigation={
          <Button
            variant="ghost"
            size="xs"
            className="-ml-2 text-muted-foreground"
            onClick={onBack}
          >
            <ChevronLeft /> Evals
          </Button>
        }
        description={
          <div className="flex flex-wrap items-center gap-x-5 gap-y-2 text-xs text-muted-foreground">
            <span className="inline-flex items-center gap-1.5">
              <Code2 aria-hidden="true" className="size-3.5" />
              {spec.agent_io ? "Saved HTTP contract" : "Runs in your code"}
            </span>
            <span>
              Agent{" "}
              <span className="font-medium text-foreground">{spec.agent}</span>
            </span>
            <span>
              Dataset{" "}
              <span className="font-medium text-foreground">
                {datasetLabel(definition.data, datasets.data ?? [])}
              </span>
            </span>
          </div>
        }
      >
        <details className="text-xs text-muted-foreground">
          <summary className="w-fit cursor-pointer hover:text-foreground">
            Scoring configuration
          </summary>
          <dl className="mt-2 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5">
            <dt>Scorers</dt>
            <dd>{spec.scorers.map(scorerLabel).join(", ")}</dd>
            <dt>Trials per case</dt>
            <dd>{spec.trials}</dd>
            <dt>Pass criteria</dt>
            <dd>{gateLabel(spec.gate)}</dd>
          </dl>
        </details>
      </LensPageHeader>
      <RunsTab
        definition={definition.data}
        datasetName={dataset?.name ?? spec.dataset_id}
        revision={spec.revision ?? dataset?.revision ?? 1}
        onOpen={onOpenRun}
      />
    </div>
  );
}
