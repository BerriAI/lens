"use client";

import { useEffect, useState, useSyncExternalStore } from "react";
import { ChevronDown, ChevronRight, Plus, RotateCcw } from "lucide-react";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "../../ui/dropdown-menu";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter } from "../../ui/dialog";
import { LensPageHeader } from "../ui/LensPageHeader";
import { useOptimizationStore } from "../data/LensServices";
import { useOptimizationRoute } from "../route";
import { resetDemoStorage } from "../data/demo/storage";
import { NewOptimization } from "./NewOptimization";
import { OptimizationDetail } from "./OptimizationDetail";
import { modes, modeLabels, recommendedModel, type Mode, type OptimizationConfig } from "./model";
import type { OptimizationStore } from "../data/demo/demoOptimizations";

const modeDescriptions = {
  live: "Test models on a share of live traffic",
  shadow: "Compare models alongside live traffic",
  offline: "Compare models using a saved dataset",
};

export function ModelOptimizations({ canView = true, readOnly = false }: { canView?: boolean; readOnly?: boolean }) {
  const store = useOptimizationStore();
  if (!canView) return <p className="lens-page-body text-sm text-muted-foreground">Model optimizations require proxy administrator access.</p>;
  return store ? (
    <OptimizationWorkspace store={store} readOnly={readOnly} />
  ) : (
    <p className="lens-page-body text-sm text-muted-foreground">Model optimizations are available in the demo.</p>
  );
}
function OptimizationWorkspace({ store, readOnly }: { store: OptimizationStore; readOnly: boolean }) {
  const state = useSyncExternalStore(store.subscribe, store.snapshot, store.snapshot);
  const route = useOptimizationRoute();
  const [initial, setInitial] = useState<OptimizationConfig>();
  const [resetting, setResetting] = useState(false);
  useEffect(() => {
    store.advance();
    const timer = setInterval(store.advance, store.live ? 5000 : 1800);
    return () => clearInterval(timer);
  }, [store]);
  const run = state.runs.find((item) => item.id === route.id);
  const openNew = (mode: Mode, config?: OptimizationConfig) => {
    setInitial(config);
    route.create(mode);
  };
  const filtered = state.runs.filter(
    (item) =>
      (route.mode === "all" || item.config.mode === route.mode) &&
      `${item.config.agent} ${item.config.datasetName} ${item.config.models.join(" ")}`
        .toLowerCase()
        .includes(route.search.toLowerCase()),
  );
  return (
    <>
      {state.error && <p role="alert" className="lens-page-body text-sm text-destructive">{state.error} <Button size="xs" variant="outline" onClick={store.advance}>Retry</Button></p>}
      {state.loading && <p role="status" className="lens-page-body text-sm text-muted-foreground">Loading optimizations…</p>}
      {run ? (
        <OptimizationDetail
          key={run.id}
          run={run}
          store={store}
          readOnly={readOnly}
          onBack={() => route.open(null)}
          onNew={(config) => openNew(config.mode, config)}
        />
      ) : (
        <section className="flex min-h-0 flex-1 flex-col">
          <LensPageHeader
            page="optimizations"
            title="Model optimizations"
            actions={
              <>
                {!store.live && <Button size="sm" variant="ghost" onClick={() => setResetting(true)}>
                  <RotateCcw />
                  Reset demo
                </Button>}
                {!readOnly && <DropdownMenu>
                  <DropdownMenuTrigger render={<Button size="sm" />}>
                    <Plus />
                    New optimization
                    <ChevronDown />
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end" className="w-80 p-1">
                    {modes.map((mode) => (
                      <DropdownMenuItem
                        key={mode}
                        className="flex flex-col items-start gap-1 px-3 py-3"
                        onClick={() => openNew(mode)}
                      >
                        <span className="font-medium">{modeLabels[mode]}</span>
                        <span className="text-xs text-muted-foreground">{modeDescriptions[mode]}</span>
                      </DropdownMenuItem>
                    ))}
                  </DropdownMenuContent>
                </DropdownMenu>}
              </>
            }
          />
          <div className="lens-page-body min-h-0 flex-1 overflow-y-auto">
            <div className="mx-auto grid max-w-[1200px] gap-5">
              <div className="flex flex-wrap items-center justify-between gap-4">
                <div role="group" aria-label="Optimization type" className="flex gap-1">
                  {(["all", ...modes] as const).map((mode) => (
                    <Button
                      size="sm"
                      key={mode}
                      variant={route.mode === mode ? "secondary" : "ghost"}
                      aria-pressed={route.mode === mode}
                      onClick={() => route.filter(mode)}
                    >
                      {mode === "all" ? "All" : modeLabels[mode]}
                    </Button>
                  ))}
                </div>
                <Input
                  aria-label="Search optimizations"
                  placeholder="Search optimizations…"
                  className="w-full sm:w-64"
                  value={route.search}
                  onChange={(e) => route.setSearch(e.target.value)}
                />
              </div>
              <div className="lens-panel overflow-x-auto">
                <table aria-label="Model optimizations" className="w-full min-w-[650px] text-left text-sm">
                  <thead className="border-b bg-muted/30 text-xs text-muted-foreground">
                    <tr>
                      {["Agent", "Type", "Status", "Recommendation", "Savings", ""].map((label, index) => (
                        <th key={index} className="px-4 py-3 font-medium">
                          {label}
                        </th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {filtered.map((item) => {
                      const best = recommendedModel(item);
                      return (
                        <tr key={item.id} className="border-b last:border-0 hover:bg-muted/30">
                          <td className="px-4 py-5">
                            <button
                              onClick={() => route.open(item.id)}
                              className="text-left font-mono text-xs font-medium hover:underline"
                            >
                              {item.config.agent}
                            </button>
                            <p className="mt-1.5 text-xs text-muted-foreground">
                              {item.config.mode === "offline"
                                ? item.config.datasetName
                                : `${item.config.traffic}% of traffic`}
                            </p>
                          </td>
                          <td className="px-4 text-xs">
                            {modeLabels[item.config.mode]}
                            {item.config.mode === "live" && (
                              <p className="mt-1.5 text-muted-foreground">
                                {item.config.judge === "feedback" ? "User feedback" : "LLM judge"}
                              </p>
                            )}
                          </td>
                          <td className="px-4 text-xs capitalize">{item.status}</td>
                          <td className="px-4">
                            <span className="font-mono text-xs">
                              {best?.id ?? (item.status === "completed" ? (store.live ? "Insufficient evidence" : "Keep current model") : "Collecting results")}
                            </span>
                            <p className="mt-1.5 text-xs text-muted-foreground">
                              {item.completed.toLocaleString()}{" "}
                              {store.live ? "model runs" : item.config.mode === "offline" ? "cases" : "sampled runs"}
                            </p>
                          </td>
                          <td className="px-4 text-sm tabular-nums">{best ? `${best.savings}%` : "—"}</td>
                          <td className="pr-3">
                            <Button
                              variant="ghost"
                              size="icon-sm"
                              aria-label={`Open ${modeLabels[item.config.mode].toLowerCase()} optimization for ${item.config.agent}`}
                              onClick={() => route.open(item.id)}
                            >
                              <ChevronRight />
                            </Button>
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
                {!filtered.length && (
                  <div className="grid gap-3 p-8 text-center">
                    <p className="text-sm text-muted-foreground">No optimizations match this view.</p>
                    <div>
                      <Button disabled={readOnly} size="sm" onClick={() => openNew(route.mode === "all" ? "live" : route.mode)}>
                        New optimization
                      </Button>
                    </div>
                  </div>
                )}
              </div>
              <p className="text-xs text-muted-foreground">
                {state.runs.length} optimizations{!store.live && " · Simulated traffic and results"}
              </p>
            </div>
          </div>
        </section>
      )}
      {route.newMode && !readOnly && (
        <NewOptimization
          key={`${route.newMode}-${route.datasetId ?? ""}-${route.revision ?? ""}`}
          mode={route.newMode}
          store={store}
          initial={initial}
          datasetId={route.datasetId}
          datasetRevision={route.revision}
          onClose={route.closeCreate}
          onCreated={(id) => {
            setInitial(undefined);
            route.open(id);
          }}
        />
      )}
      <Dialog open={resetting} onOpenChange={setResetting}>
        <DialogContent className="lens-shell-overlay" aria-describedby={undefined}>
          <DialogHeader>
            <DialogTitle>Reset demo?</DialogTitle>
          </DialogHeader>
          <p className="text-sm text-muted-foreground">
            Restore the completed examples and clear your demo comparisons, connections, and dataset edits.
          </p>
          <DialogFooter>
            <Button variant="outline" onClick={() => setResetting(false)}>
              Cancel
            </Button>
            <Button
              onClick={() => {
                resetDemoStorage();
                window.location.assign(`${window.location.pathname}?demo=true&tab=optimizations`);
              }}
            >
              Reset demo
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
