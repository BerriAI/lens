"use client";

import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { Activity, LoaderCircle, Settings2 } from "lucide-react";
import { useNow } from "../../../hooks/useNow";
import { SearchSelect } from "../../shared/SearchSelect";
import { Button } from "../../ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "../../ui/dialog";
import { Input } from "../../ui/input";
import { Switch } from "../../ui/switch";
import { Textarea } from "../../ui/textarea";
import { useLensApi } from "../data/LensServices";
import { useSaveLens } from "../data/mutations";
import { useAnalysisModels } from "../setup/fields/useAnalysisModels";
import { analysisModelOptions, modelGate } from "../setup/fields/analysisModels";
import { useLensRoute } from "../route";
import type { Lens, Settings } from "../model/types";

function AnalysisSettings({ lens, onClose }: { lens: Lens; onClose: () => void }) {
  const models = useAnalysisModels();
  const save = useSaveLens();
  const [settings, setSettings] = useState<Settings>(lens.settings);
  const gate = modelGate(models, settings.model, settings.model === lens.settings.model);
  const pausingSavedModel = !settings.enabled && settings.model === lens.settings.model;
  const valid =
    (gate.modelValid || pausingSavedModel) &&
    Number.isSafeInteger(settings.interval_minutes) &&
    settings.interval_minutes > 0 &&
    (settings.context.trim() || settings.checks?.some((check) => check.enabled));
  return (
    <Dialog open onOpenChange={(open) => !open && !save.isPending && onClose()}>
      <DialogContent className="sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>Analysis for {lens.settings.agent_name || "all agents"}</DialogTitle>
          <DialogDescription>Review new traces on a schedule. Changes apply to the next run.</DialogDescription>
        </DialogHeader>
        <div className="space-y-5">
          <div className="flex items-center justify-between text-sm font-medium">
            <label htmlFor="analysis-enabled">Automatic analysis</label>
            <Switch
              id="analysis-enabled"
              checked={settings.enabled}
              onCheckedChange={(enabled) => setSettings({ ...settings, enabled })}
            />
          </div>
          <div className="space-y-2">
            <label className="text-sm font-medium" htmlFor="analysis-model">
              Model
            </label>
            <SearchSelect
              inputId="analysis-model"
              aria-label="Analysis model"
              options={analysisModelOptions(models.models, models.modelDetails)}
              value={settings.model}
              onValueChange={(model) => model && setSettings({ ...settings, model })}
              allowClear={false}
            />
            {(gate.unavailable || gate.unsupported) && (
              <p role="status" className="text-xs text-muted-foreground">
                This model is unavailable for analysis. Choose another model or pause automatic analysis.
              </p>
            )}
            {models.modelsError && (
              <p role="alert" className="text-xs text-destructive">
                Could not load models: {models.modelsError}
              </p>
            )}
          </div>
          <div className="space-y-2">
            <label htmlFor="analysis-context" className="text-sm font-medium">
              What to look for
            </label>
            <Textarea
              id="analysis-context"
              rows={5}
              value={settings.context}
              onChange={(event) => setSettings({ ...settings, context: event.target.value })}
            />
          </div>
          {settings.checks?.map((check, index) => (
            <div key={check.id} className="space-y-2">
              <label htmlFor={`analysis-check-${index}`} className="text-sm font-medium">
                Additional instruction {index + 1}
                {!check.enabled && " (paused)"}
              </label>
              <Textarea
                id={`analysis-check-${index}`}
                value={check.instruction}
                onChange={(event) =>
                  setSettings({
                    ...settings,
                    checks: settings.checks?.map((item, at) =>
                      at === index ? { ...item, instruction: event.target.value } : item,
                    ),
                  })
                }
              />
            </div>
          ))}
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2">
              <label htmlFor="analysis-frequency" className="text-sm font-medium">
                Check every (minutes)
              </label>
              <Input
                id="analysis-frequency"
                type="number"
                min={1}
                value={settings.interval_minutes}
                onChange={(event) =>
                  setSettings({
                    ...settings,
                    interval_minutes: Number(event.target.value),
                  })
                }
              />
            </div>
            <div className="space-y-2">
              <label htmlFor="analysis-budget" className="text-sm font-medium">
                Monthly budget (USD)
              </label>
              <Input
                id="analysis-budget"
                type="number"
                min={0.01}
                step={0.01}
                value={settings.monthly_budget}
                onChange={(event) =>
                  setSettings({
                    ...settings,
                    monthly_budget: Number(event.target.value),
                  })
                }
              />
            </div>
          </div>
          <p className="text-xs leading-5 text-muted-foreground">
            The first automatic run waits for 10 traces. Later runs review new traces from the last{" "}
            {settings.lookback_hours} hours, up to {settings.sample_size ?? "all"} per run. Recent traces settle for 2
            minutes before analysis.
          </p>
        </div>
        {save.error && (
          <p role="alert" className="text-sm text-destructive">
            {save.error.message}
          </p>
        )}
        <DialogFooter>
          <Button variant="outline" disabled={save.isPending} onClick={onClose}>
            Cancel
          </Button>
          <Button
            disabled={
              !valid || !Number.isFinite(settings.monthly_budget) || settings.monthly_budget <= 0 || save.isPending
            }
            onClick={() => save.mutate({ id: lens.id, settings }, { onSuccess: onClose })}
          >
            {save.isPending ? "Saving…" : "Save changes"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function AnalysisRow({
  lens,
  readOnly,
  ready,
  availableModels,
}: {
  lens: Lens;
  readOnly: boolean;
  ready: boolean;
  availableModels: readonly string[] | null;
}) {
  const api = useLensApi();
  const now = useNow(30_000);
  const [editing, setEditing] = useState(false);
  const job = lens.jobs?.[0];
  const active = lens.jobs?.find((item) => item.status === "running" || item.status === "queued");
  const modelUnavailable =
    availableModels !== null && !availableModels.includes(active?.settings.model ?? lens.settings.model);
  const running = active?.status === "running";
  const first = !job && !lens.last_scan_at && lens.id.startsWith("auto-agent-");
  const sample = useQuery({
    queryKey: ["lens", "automatic-sample", api.scope, lens.id, lens.revision],
    queryFn: () => api.sample(lens.settings, 0, new Date().toISOString()),
    enabled: first && lens.settings.enabled && ready && !modelUnavailable,
    refetchInterval: 30_000,
  });
  const waiting = Math.max(0, 10 - (sample.data?.eligible ?? 0));
  const status =
    active && (running || !modelUnavailable)
      ? active.status === "running"
        ? "Running"
        : "Queued"
      : !lens.settings.enabled
        ? "Paused"
        : modelUnavailable
          ? "Selected model unavailable"
          : !ready
            ? "Waiting for connection"
            : first
              ? sample.error
                ? "Trace count unavailable"
                : sample.isPending
                  ? "Checking traces"
                  : waiting
                    ? `Waiting for traces · ${sample.data?.eligible ?? 0}/10`
                    : "Ready for first analysis"
              : job?.status === "failed"
                ? "Last analysis failed"
                : "Scheduled";
  const next = !lens.settings.enabled
    ? "Enable analysis to resume"
    : modelUnavailable
      ? "Configure another model or pause analysis. Your saved model has not been changed."
      : !ready
        ? "Connect an analysis model in Settings"
        : first
          ? sample.error
            ? "Retrying the trace count automatically"
            : sample.isPending
              ? "Checking received traces"
              : `${waiting} more ${waiting === 1 ? "trace" : "traces"} before the first run`
          : Date.parse(lens.next_run_at) <= now
            ? "Next run: waiting for worker"
            : `Next run: ${new Date(lens.next_run_at).toLocaleString()}`;
  return (
    <div className="flex flex-wrap items-center justify-between gap-3 border-t px-4 py-3 text-xs">
      <div className="min-w-0 space-y-1">
        <p className="font-medium text-foreground">{lens.settings.agent_name || "All agents"}</p>
        <p className="max-w-lg truncate font-mono text-[11px] text-muted-foreground" title={lens.settings.model}>
          {lens.settings.model} · every {lens.settings.interval_minutes} min
        </p>
      </div>
      <div className="min-w-0 flex-1 space-y-1 sm:pl-6">
        <p role="status" className="flex items-center gap-2 font-medium">
          {active && (running || !modelUnavailable) ? (
            <LoaderCircle aria-hidden className="size-3.5 motion-safe:animate-spin" />
          ) : (
            <Activity aria-hidden className="size-3.5 text-muted-foreground" />
          )}
          {status}
        </p>
        <p className="text-muted-foreground">{active && (running || !modelUnavailable) ? active.stage : next}</p>
        {job?.finished_at && (
          <p className="text-muted-foreground">
            Last result:{" "}
            {job.status === "failed"
              ? "failed"
              : job.status === "cancelled"
                ? "cancelled"
                : `${job.findings?.length ?? 0} findings`}{" "}
            · {new Date(job.finished_at).toLocaleString()}
          </p>
        )}
        {job?.error && (
          <p role="alert" className="max-w-2xl break-words text-destructive">
            {job.error}
          </p>
        )}
        {sample.error && (
          <p role="alert" className="text-destructive">
            {sample.error.message}
          </p>
        )}
      </div>
      {!readOnly && (
        <Button
          variant="outline"
          size="sm"
          aria-label={`Configure analysis for ${lens.settings.agent_name || "all agents"}`}
          onClick={() => setEditing(true)}
        >
          <Settings2 className="size-3.5" />
          Configure
        </Button>
      )}
      {editing && <AnalysisSettings lens={lens} onClose={() => setEditing(false)} />}
    </div>
  );
}

export function AutomaticAnalysis({ lenses, readOnly, ready }: { lenses: Lens[]; readOnly: boolean; ready: boolean }) {
  const { setTab, demo } = useLensRoute();
  const models = useAnalysisModels();
  const availableModels = !demo && !models.modelsLoading && !models.modelsError ? models.models : null;
  return (
    <section
      aria-label="Automatic analysis"
      className="mb-4 max-h-72 shrink-0 overflow-auto rounded-md border bg-background"
    >
      <div className="flex flex-wrap items-center justify-between gap-2 px-4 py-3">
        <h2 className="text-sm font-medium">Automatic analysis</h2>
        <p className="text-xs text-muted-foreground">Starts after 10 traces per agent</p>
      </div>
      {!ready && !readOnly && (
        <div className="px-4 pb-3">
          <Button variant="outline" size="sm" onClick={() => setTab("settings")}>
            Configure analysis
          </Button>
        </div>
      )}
      {lenses.length ? (
        lenses.map((lens) => (
          <AnalysisRow key={lens.id} lens={lens} readOnly={readOnly} ready={ready} availableModels={availableModels} />
        ))
      ) : (
        <p role="status" className="border-t px-4 py-4 text-sm text-muted-foreground">
          {ready
            ? "Waiting for traces. Each agent is added here automatically."
            : "Waiting for connection. Configure an analysis model in Settings to get started."}
        </p>
      )}
    </section>
  );
}
