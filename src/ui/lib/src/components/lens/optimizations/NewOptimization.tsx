"use client";

import { useState, type ReactNode } from "react";
import { Check, Loader2 } from "lucide-react";
import { Button } from "../../ui/button";
import { Checkbox } from "../../ui/checkbox";
import { Input } from "../../ui/input";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter } from "../../ui/dialog";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../ui/select";
import { useDataset, useDatasets } from "../datasets/api";
import {
  baselineFor,
  catalog,
  defaultConfig,
  demoAgents,
  modelEligibility,
  modeLabels,
  presetModels,
  type Mode,
  type OptimizationConfig,
  type Tier,
} from "./model";
import type { OptimizationStore } from "../data/demo/demoOptimizations";

export function Choice({
  label,
  value,
  items,
  onChange,
}: {
  label: string;
  value: string;
  items: { value: string; label: string }[];
  onChange(value: string): void;
}) {
  return (
    <Select value={value || null} items={items} onValueChange={(value: string | null) => value && onChange(value)}>
      <SelectTrigger className="w-full" aria-label={label}>
        <SelectValue placeholder={`Select ${label.toLowerCase()}`} />
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
function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="grid gap-2">
      <div className="text-sm font-medium">{label}</div>
      {children}
    </div>
  );
}
export function NewOptimization({
  mode,
  store,
  initial,
  datasetId,
  datasetRevision,
  onClose,
  onCreated,
}: {
  mode: Mode;
  store: OptimizationStore;
  initial?: OptimizationConfig;
  datasetId?: string | null;
  datasetRevision?: number | null;
  onClose(): void;
  onCreated(id: string): void;
}) {
  const datasets = useDatasets();
  const [draft, setDraft] = useState(() => ({
    ...defaultConfig(mode),
    ...initial,
    mode,
    datasetId: datasetId ?? initial?.datasetId ?? "",
    revision: datasetRevision ?? initial?.revision ?? 0,
  }));
  const [preset, setPreset] = useState<Tier | "Auto" | "Custom">(initial ? "Custom" : "Auto");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [connecting, setConnecting] = useState(false);
  const [endpoint, setEndpoint] = useState("https://runner.example.com/evaluate");
  const dataset = datasets.data?.find((item) => item.id === draft.datasetId) ?? datasets.data?.[0];
  const selectedDataset = useDataset(mode === "offline" ? (dataset?.id ?? null) : null, draft.revision || null);
  const agent = mode === "offline" ? dataset?.agent_name || draft.agent : draft.agent;
  const models =
    preset === "Custom"
      ? draft.models.filter((id) => !modelEligibility(agent, id) && id !== baselineFor(agent))
      : presetModels(agent, preset);
  const connected = store.snapshot().connectedAgents.includes(agent);
  const set = (next: Partial<OptimizationConfig>) => setDraft((current) => ({ ...current, ...next }));
  const selectModel = (id: string, checked: boolean) => {
    setPreset("Custom");
    set({
      models: checked ? [...models, id] : models.filter((model) => model !== id),
    });
  };
  const connect = () => {
    try {
      const url = new URL(endpoint);
      if (!["https:", "http:"].includes(url.protocol) || url.username || url.password) throw new Error();
      store.connect(agent);
      setConnecting(false);
      setError("");
    } catch {
      setError("Enter a runner URL, such as https://runner.example.com/evaluate.");
    }
  };
  const submit = async () => {
    if (mode === "offline" && !selectedDataset.data) return setError("Choose a dataset first.");
    setBusy(true);
    setError("");
    try {
      const id = await store.create({
        ...draft,
        agent,
        models,
        judge: mode === "live" ? draft.judge : "llm",
        datasetId: dataset?.id ?? "",
        datasetName: dataset?.name ?? "",
        revision: selectedDataset.data?.revision ?? 1,
      });
      onCreated(id);
    } catch (error) {
      setError(error instanceof Error ? error.message : "Couldn't start this comparison.");
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent
        className="lens-shell-overlay max-h-[90dvh] overflow-y-auto sm:max-w-xl"
        aria-describedby={undefined}
      >
        <DialogHeader>
          <DialogTitle>New {modeLabels[mode].toLowerCase()} optimization</DialogTitle>
        </DialogHeader>
        <form
          className="grid gap-5"
          onSubmit={(event) => {
            event.preventDefault();
            void submit();
          }}
        >
          <div className={mode === "offline" ? "grid gap-4" : "grid grid-cols-[1fr_110px] gap-4"}>
            {mode === "offline" ? (
              <Field label="Dataset">
                <Choice
                  label="Dataset"
                  value={dataset?.id ?? ""}
                  items={(datasets.data ?? []).map((d) => ({
                    value: d.id,
                    label: `${d.name} · ${d.case_count} cases`,
                  }))}
                  onChange={(datasetId) => set({ datasetId, revision: 0 })}
                />
              </Field>
            ) : (
              <>
                <Field label="Agent">
                  <Choice
                    label="Agent"
                    value={agent}
                    items={demoAgents.map((name) => ({
                      value: name,
                      label: name,
                    }))}
                    onChange={(agent) => set({ agent, models: presetModels(agent, "Auto") })}
                  />
                </Field>
                <Field label="Traffic">
                  <div className="relative">
                    <Input
                      aria-label="Traffic percent"
                      type="number"
                      min={1}
                      max={100}
                      required
                      value={draft.traffic}
                      className="pr-7"
                      onChange={(event) => set({ traffic: Number(event.target.value) })}
                    />
                    <span className="absolute right-3 top-2 text-muted-foreground">%</span>
                  </div>
                </Field>
              </>
            )}
          </div>
          <p className="-mt-3 text-xs text-muted-foreground">
            Current model <span className="font-mono text-foreground">{baselineFor(agent)}</span>
          </p>
          {mode === "shadow" && (
            <p className="text-xs text-muted-foreground">
              Shadow tests replay the final model call of each sampled trace.
            </p>
          )}
          {mode === "offline" && (
            <Field label="Run">
              <Choice
                label="Replay scope"
                value={draft.scope}
                items={[
                  { value: "final", label: "Final model call" },
                  { value: "agent", label: "Full agent" },
                ]}
                onChange={(scope) => set({ scope: scope === "agent" ? "agent" : "final" })}
              />
              {draft.scope === "final" ? (
                <p className="text-xs text-muted-foreground">Uses the recorded conversation and tool results.</p>
              ) : connected ? (
                <p className="flex items-center gap-2 text-xs text-muted-foreground">
                  <Check className="size-3.5 text-emerald-600" />
                  {agent} connected · model override supported
                </p>
              ) : (
                <div className="rounded-lg border p-3">
                  {connecting ? (
                    <div className="grid gap-3">
                      <label htmlFor="runner-url" className="text-xs font-medium">
                        Runner URL
                      </label>
                      <Input id="runner-url" value={endpoint} onChange={(event) => setEndpoint(event.target.value)} />
                      <div className="flex items-center justify-between gap-3">
                        <p className="text-xs text-muted-foreground">Connection is simulated in this demo.</p>
                        <Button size="sm" type="button" onClick={connect}>
                          Connect runner
                        </Button>
                      </div>
                    </div>
                  ) : (
                    <div className="flex items-center justify-between gap-4">
                      <p className="text-xs text-muted-foreground">Run your agent with its tools and model override.</p>
                      <Button size="sm" variant="outline" type="button" onClick={() => setConnecting(true)}>
                        Connect agent
                      </Button>
                    </div>
                  )}
                </div>
              )}
            </Field>
          )}
          <Field label="Models">
            <div className="flex flex-wrap gap-1.5" role="group" aria-label="Model selection">
              {(["Auto", "Frontier", "Balanced", "Flash", "Custom"] as const).map((tier) => (
                <Button
                  type="button"
                  size="sm"
                  key={tier}
                  variant={preset === tier ? "default" : "outline"}
                  aria-pressed={preset === tier}
                  onClick={() => {
                    set({ models });
                    setPreset(tier);
                  }}
                >
                  {tier}
                </Button>
              ))}
            </div>
            <div className="max-h-44 overflow-y-auto rounded-lg border py-1">
              {catalog
                .filter(
                  (model) =>
                    model.id !== baselineFor(agent) &&
                    !modelEligibility(agent, model.id) &&
                    (preset === "Custom" || models.includes(model.id)),
                )
                .map((model) => (
                  <label key={model.id} className="flex cursor-pointer items-center gap-3 px-3 py-2 hover:bg-muted/40">
                    <Checkbox
                      checked={models.includes(model.id)}
                      onCheckedChange={(checked) => selectModel(model.id, checked === true)}
                      aria-label={model.id}
                    />
                    <span className="font-mono text-xs">{model.id}</span>
                    <span className="ml-auto text-xs text-muted-foreground">{model.tier}</span>
                  </label>
                ))}
            </div>
            <details className="text-xs text-muted-foreground">
              <summary className="cursor-pointer">
                {catalog.filter((model) => modelEligibility(agent, model.id)).length} models unavailable for this agent
              </summary>
              <div className="mt-2 grid gap-2">
                {catalog
                  .filter((model) => modelEligibility(agent, model.id))
                  .map((model) => (
                    <div key={model.id}>
                      <span className="font-mono">{model.id}</span> · {modelEligibility(agent, model.id)}
                    </div>
                  ))}
              </div>
            </details>
          </Field>
          <Field label="Quality">
            {mode === "live" ? (
              <Choice
                label="Quality measurement"
                value={draft.judge}
                items={[
                  { value: "feedback", label: "User feedback · 0–10" },
                  { value: "llm", label: "LLM judge" },
                ]}
                onChange={(judge) => set({ judge: judge === "feedback" ? "feedback" : "llm" })}
              />
            ) : (
              <p className="text-sm">
                LLM judge{" "}
                <span className="ml-2 text-xs text-muted-foreground">Correctness, completeness, and groundedness</span>
              </p>
            )}
            {(mode !== "live" || draft.judge === "llm") && (
              <p className="font-mono text-xs text-muted-foreground">gpt-6-astra · blind comparison</p>
            )}
            <details className="text-xs text-muted-foreground">
              <summary className="cursor-pointer">Advanced</summary>
              <div className="mt-3 grid grid-cols-2 gap-3">
                {mode === "live" && draft.judge === "llm" && (
                  <>
                    <label className="grid gap-2">
                      Judge baseline %
                      <Input
                        aria-label="Judge baseline percent"
                        type="number"
                        min={1}
                        max={100}
                        value={draft.baselineSampling}
                        onChange={(e) => set({ baselineSampling: Number(e.target.value) })}
                      />
                    </label>
                    <label className="grid gap-2">
                      Judge test traffic %
                      <Input
                        aria-label="Judge test traffic percent"
                        type="number"
                        min={1}
                        max={100}
                        value={draft.candidateSampling}
                        onChange={(e) => set({ candidateSampling: Number(e.target.value) })}
                      />
                    </label>
                  </>
                )}
                <label className="grid gap-2">
                  Allowed quality difference
                  <Input
                    aria-label="Allowed quality difference"
                    type="number"
                    min={0}
                    max={1}
                    step={0.1}
                    value={draft.tolerance}
                    onChange={(e) => set({ tolerance: Number(e.target.value) })}
                  />
                </label>
              </div>
            </details>
          </Field>
          {error && (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          )}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={onClose}>
              Cancel
            </Button>
            <Button
              type="submit"
              disabled={
                busy ||
                !models.length ||
                (mode === "offline" && (!selectedDataset.data || (draft.scope === "agent" && !connected)))
              }
            >
              {busy && <Loader2 className="size-4 animate-spin" />}
              {mode === "offline" ? "Run comparison" : `Start ${mode} test`}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
