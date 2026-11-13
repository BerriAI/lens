"use client";

import { useState } from "react";
import { ArrowLeft, ChevronRight, Download, Pause, Play, RotateCcw } from "lucide-react";
import { Button } from "../../ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "../../ui/dialog";
import { LensPageHeader } from "../ui/LensPageHeader";
import { useOpenSourceTrace } from "../route";
import { Choice } from "./NewOptimization";
import { CostQualityPlot } from "./CostQualityPlot";
import { RunnerSetup } from "./RunnerSetup";
import {
  baselineFor,
  comparisonRows,
  exampleJudgment,
  modeLabels,
  recommendedModel,
  type ComparisonExample,
  type Optimization,
  type OptimizationConfig,
} from "./model";
import type { OptimizationStore } from "../data/demo/demoOptimizations";

const money = (value: number) => Number.isFinite(value) ? `$${value.toLocaleString("en-US", { minimumFractionDigits: 4, maximumFractionDigits: 8 })}` : "—";
const number = (value: number) => value.toLocaleString("en-US");
export function readableOutput(raw: string): string {
  try {
    const value: unknown = JSON.parse(raw);
    if (Array.isArray(value))
      return (
        value
          .map((item: { content?: unknown; text?: unknown }) =>
            typeof item.content === "string" ? item.content : typeof item.text === "string" ? item.text : "",
          )
          .filter(Boolean)
          .join("\n") || raw
      );
  } catch {
    return raw;
  }
  return raw;
}
function candidateOutput(example: ComparisonExample, model: string) {
  const result = exampleJudgment(example, model);
  if (result.answer) return result.answer;
  return readableOutput(example.candidate || example.original)
    .replace(
      "Yes. Your order is eligible for a return. I can help you start it.",
      "Your order qualifies for a return. I can help you start the process.",
    )
    .replace(
      "Keep the original packaging; I can help create a free return label.",
      "Please keep the original packaging. I can arrange a free return label.",
    )
    .replace("Your refund was issued yesterday. It should appear", "The refund was sent yesterday and should appear")
    .replace("Please confirm the new delivery address.", "Could you confirm the new delivery address?")
    .replace("The carrier has not reported any delays.", "There are no delays reported by the carrier.");
}

export function OptimizationDetail({
  run,
  store,
  onBack,
  onNew,
  readOnly = false,
}: {
  run: Optimization;
  store: OptimizationStore;
  readOnly?: boolean;
  onBack(): void;
  onNew(config: OptimizationConfig): void;
}) {
  const rows = comparisonRows(run);
  const recommended = recommendedModel(run);
  const [model, setModel] = useState(recommended?.id ?? run.config.models[0]);
  const [example, setExample] = useState<ComparisonExample | null>(null);
  const [query, setQuery] = useState("");
  const [exampleLimit, setExampleLimit] = useState(8);
  const baseline = rows[0];
  const isReplay = run.config.mode !== "live";
  const unit = isReplay && run.config.scope === "final" ? "final call" : "run";
  const openTrace = useOpenSourceTrace();
  const observed = run.completed > 0;
  const title = recommended
    ? `${isReplay ? "Try" : "Move to"} ${recommended.id}${isReplay ? " in live traffic" : ""}`
    : observed && run.status === "completed"
      ? store.live ? "No supported recommendation yet" : "Keep the current model"
      : "Gathering comparison results";
  const download = () => {
    const cell = (value: string | number) => {
      const text = typeof value === "number" && !Number.isFinite(value) ? "" : String(value);
      return `"${(/^[=+@\-\t\r]/.test(text) ? "'" : "") + text.replaceAll('"', '""')}"`;
    };
    const csv = [
      "model,runs,scored,quality,cost_per_unit,latency_seconds,savings_percent",
      ...rows.map((row) => [row.id, row.count, row.scored, row.quality, row.cost, row.latency, row.savings].map(cell).join(",")),
    ].join("\n");
    const url = URL.createObjectURL(new Blob([csv], { type: "text/csv" }));
    const link = document.createElement("a");
    link.href = url;
    link.download = `${run.config.mode}-model-comparison.csv`;
    link.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  };
  return (
    <section className="flex min-h-0 flex-1 flex-col">
      <LensPageHeader
        title={`${modeLabels[run.config.mode]} optimization`}
        navigation={
          <Button size="xs" variant="ghost" onClick={onBack}>
            <ArrowLeft />
            Model optimizations
          </Button>
        }
        actions={
          <>
            <Button size="sm" variant="outline" onClick={download}>
              <Download />
              Export
            </Button>
            {!readOnly && run.status !== "completed" && (
              <Button
                size="sm"
                variant="outline"
                onClick={() => store.setStatus(run.id, run.status === "paused" ? "running" : "paused")}
              >
                {run.status === "paused" ? <Play /> : <Pause />}
                {run.status === "paused" ? "Resume" : "Pause"}
              </Button>
            )}
            {!readOnly && run.status !== "completed" && run.config.mode !== "offline" && (
              <Button size="sm" variant="outline" onClick={() => store.setStatus(run.id, "completed")}>
                Finish test
              </Button>
            )}
            <Button disabled={readOnly} size="sm" variant="outline" onClick={() => onNew(run.config)}>
              <RotateCcw />
              Run again
            </Button>
            {!readOnly && isReplay && recommended && (
              <Button
                size="sm"
                onClick={() =>
                  onNew({
                    ...run.config,
                    mode: "live",
                    judge: "feedback",
                    models: [recommended.id],
                  })
                }
              >
                Start live test
              </Button>
            )}
          </>
        }
      />
      <div className="lens-page-body min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto grid w-full max-w-[1200px] gap-6">
          {run.error && <p role="alert" className="text-sm text-destructive">{run.error}</p>}
          {store.live && run.config.mode === "live" && <RunnerSetup run={run} />}
          {store.live && run.config.mode === "offline" && run.config.scope === "agent" && <RunnerSetup run={run} />}
          <div className="grid gap-3 border-b pb-6">
            <h2 className="text-xl font-normal tracking-tight sm:text-2xl">
              {recommended ? (
                <>
                  {isReplay ? "Try " : "Move to "}
                  <strong className="font-semibold">{recommended.id}</strong>
                  {isReplay ? " in live traffic" : ""}
                </>
              ) : (
                title
              )}
            </h2>
            {recommended ? (
              <p className="text-sm text-muted-foreground">
                <strong className="font-medium text-foreground">
                  {recommended.savings}% {isReplay ? "estimated " : ""}lower cost per {unit}
                </strong>
                {" · "}
                {run.config.judge === "feedback" ? "Feedback" : "Judge score"} {recommended.quality.toFixed(2)} vs.{" "}
                {baseline.quality.toFixed(2)} / 10
                {" · "}
                {number(recommended.scored)} scored {unit}s
              </p>
            ) : (
              <p className="text-sm text-muted-foreground">
                {run.status === "completed"
                  ? "No cheaper model met the quality tolerance with enough scored examples."
                  : store.live ? "Collecting enough evidence to compare quality and cost." : "A recommendation appears after 30 scored examples per model."}
              </p>
            )}
            {!readOnly && run.status !== "completed" && (
              <div className="grid gap-2">
                {run.config.mode === "offline" && (
                  <div className="h-1.5 overflow-hidden rounded-full bg-muted">
                    <div
                      className="h-full bg-primary transition-all"
                      style={{
                        width: `${run.target ? (run.completed / run.target) * 100 : 0}%`,
                      }}
                    />
                  </div>
                )}
                <p role="status" className="text-xs text-muted-foreground">
                  {number(run.completed)}
                  {run.config.mode === "offline" ? ` / ${number(run.target)} ${store.live ? "model runs" : "cases"}` : store.live ? " model runs" : " sampled runs"}
                  {run.status === "paused" ? " · paused" : " · evaluating"}
                </p>
              </div>
            )}
          </div>
          <div className="lens-panel overflow-x-auto">
            <table className="w-full text-left text-sm" aria-label="Model comparison">
              <thead className="border-b bg-muted/30 text-xs text-muted-foreground">
                <tr>
                  {[
                    "Model",
                    "Runs",
                    "Scored",
                    run.config.judge === "feedback" ? "Feedback / 10" : "Judge / 10",
                    `Cost / ${unit}`,
                    "Latency",
                    "Savings",
                  ].map((label, i) => (
                    <th key={label} className={`whitespace-nowrap px-4 py-3 font-medium ${i ? "text-right" : ""}`}>
                      {label}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {rows.map((row) => (
                  <tr
                    key={row.id}
                    className={`border-b last:border-0 ${recommended?.id === row.id ? "bg-primary/5" : ""}`}
                  >
                    <td className="px-4 py-4">
                      <span className="font-mono text-xs">{row.id}</span>
                      <div className="mt-1 text-xs text-muted-foreground">
                        {row.isBaseline
                          ? "Current model"
                          : row.scored < 30
                            ? "Needs more data"
                            : row.qualifies
                              ? recommended?.id === row.id
                                ? "Recommended"
                                : "Within quality tolerance"
                              : store.live ? "No supported recommendation" : "Quality below tolerance"}
                      </div>
                    </td>
                    <td className="px-4 text-right tabular-nums">{number(row.count)}</td>
                    <td className="px-4 text-right tabular-nums text-muted-foreground">{number(row.scored)}</td>
                    <td className="px-4 text-right tabular-nums">{row.scored ? row.quality.toFixed(2) : "—"}</td>
                    <td className="px-4 text-right tabular-nums">{observed ? money(row.cost) : "—"}</td>
                    <td className="px-4 text-right tabular-nums">{observed && Number.isFinite(row.latency) ? `${row.latency.toFixed(1)}s` : "—"}</td>
                    <td className="px-4 text-right tabular-nums">
                      {row.isBaseline || !observed || !Number.isFinite(row.savings) ? "—" : `${row.savings}%`}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            <p className="border-t px-4 py-3 text-xs text-muted-foreground">
              Quality tolerance: {run.config.tolerance.toFixed(1)} points ·{" "}
              {run.config.judge === "feedback"
                ? `Feedback coverage: ${Math.round(rows.reduce((total, row) => total + row.scored, 0) / Math.max(1, rows.reduce((total, row) => total + row.count, 0)) * 100)}%`
                : `Judge: ${run.config.judgeModel ?? "gpt-6-astra"}${run.config.mode === "live" ? ` · ${run.config.baselineSampling}% baseline / ${run.config.candidateSampling}% test traffic` : " · blind comparison"}`}
            </p>
          </div>
          {run.measured && <p className="text-xs text-muted-foreground">{run.measured.filter((result) => result.error).length} execution errors · {run.measured.filter((result) => result.scoreError).length} judge errors</p>}
          {observed && <CostQualityPlot run={run} unit={unit} />}
          {observed && (
            <section className="grid gap-3">
              <div className="flex flex-wrap items-center justify-between gap-3">
                <h3 className="text-sm font-medium">{run.config.mode === "offline" ? "Cases" : "Example runs"}</h3>
                <div className="w-full sm:w-64">
                  <Choice
                    label="Compare model"
                    value={model}
                    items={run.config.models.map((id) => ({
                      value: id,
                      label: id,
                    }))}
                    onChange={setModel}
                  />
                </div>
              </div>
              <input
                aria-label="Search comparison examples"
                className="h-9 w-full rounded-md border bg-transparent px-3 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
                placeholder="Search examples…"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
              />
              <div className="lens-panel divide-y overflow-hidden">
                {run.examples
                  .filter((item) => item.question.toLowerCase().includes(query.toLowerCase()))
                  .slice(0, exampleLimit)
                  .map((item, index) => (
                    <button
                      type="button"
                      key={item.id}
                      onClick={() => setExample(item)}
                      className="flex w-full items-center gap-4 px-4 py-3 text-left hover:bg-muted/40 focus-visible:outline-ring"
                    >
                      <span className="text-xs tabular-nums text-muted-foreground">
                        {String(index + 1).padStart(2, "0")}
                      </span>
                      <span className="min-w-0 flex-1 truncate text-sm">{readableOutput(item.question)}</span>
                      <span className="hidden text-xs text-muted-foreground sm:block">
                        {exampleJudgment(item, model).label}
                      </span>
                      <ChevronRight className="size-3.5 text-muted-foreground" />
                    </button>
                  ))}
                {!run.examples.some((item) => item.question.toLowerCase().includes(query.toLowerCase())) && (
                  <p className="p-4 text-sm text-muted-foreground">No matching examples.</p>
                )}
              </div>
              {run.examples.filter((item) => item.question.toLowerCase().includes(query.toLowerCase())).length >
                exampleLimit && (
                <div>
                  <Button size="sm" variant="outline" onClick={() => setExampleLimit((count) => count + 20)}>
                    Show more examples
                  </Button>
                </div>
              )}
            </section>
          )}
          {isReplay && (
            <p className="text-xs text-muted-foreground">
              Savings compare recorded serving cost with candidate serving cost. Replay and judge spend are additional.{" "}
              {run.config.scope === "final" && "Earlier agent steps are unchanged."}
            </p>
          )}
        </div>
      </div>
      <Dialog open={example !== null} onOpenChange={(open) => !open && setExample(null)}>
        <DialogContent
          className="lens-shell-overlay max-h-[90dvh] overflow-y-auto sm:max-w-4xl"
          aria-describedby={undefined}
        >
          {example && (
            <>
              <DialogHeader>
                <DialogTitle className="pr-6 leading-6">{readableOutput(example.question)}</DialogTitle>
              </DialogHeader>
              <div className="grid gap-4 sm:grid-cols-2">
                {[
                  {
                    label: run.config.baseline ?? baselineFor(run.config.agent),
                    text: readableOutput(example.results ? example.results[run.config.baseline ?? ""]?.output ?? "This model did not produce an output for this run." : example.original),
                  },
                  { label: model, text: candidateOutput(example, model) },
                ].map((answer) => (
                  <div key={answer.label} className="rounded-lg border p-4">
                    <h3 className="mb-3 font-mono text-xs text-muted-foreground">{answer.label}</h3>
                    <p className="whitespace-pre-wrap text-sm leading-6">{answer.text}</p>
                  </div>
                ))}
              </div>
              <p className="text-sm">{exampleJudgment(example, model).reason}</p>
              <details className="text-sm">
                <summary className="cursor-pointer font-medium">
                  {store.live ? "Recorded input" : run.config.scope === "agent" ? "Agent execution" : "Shared trace context"}
                </summary>
                {run.config.scope === "agent" && !store.live && (
                  <p className="mt-3 text-xs text-muted-foreground">
                    Input received · tools executed in simulated environment · final response generated
                  </p>
                )}
                <pre className="mt-3 max-h-52 overflow-auto whitespace-pre-wrap rounded-md bg-muted/40 p-3 text-xs leading-5">
                  {example.context}
                </pre>
              </details>
              {example.traceId && (
                <div>
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() => {
                      setExample(null);
                      openTrace({
                        agent: run.config.agent,
                        traceId: example.traceId,
                        traceRef: "",
                        spanId: example.spanId,
                      });
                    }}
                  >
                    Open source trace
                  </Button>
                </div>
              )}
            </>
          )}
        </DialogContent>
      </Dialog>
    </section>
  );
}
