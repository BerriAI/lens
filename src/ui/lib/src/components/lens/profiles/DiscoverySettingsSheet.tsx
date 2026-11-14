"use client";

import { useId, useRef, useState, type KeyboardEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { RefreshCw } from "lucide-react";
import { SearchSelect } from "../../shared/SearchSelect";
import { StatusDot } from "../../shared/StatusDot";
import type { TimeWindow } from "../../shared/timeRange/timeRange";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "../../ui/sheet";
import { Switch } from "../../ui/switch";
import { useLensApi, useProfilesApi } from "../data/LensServices";
import {
  BLOCKER_TEXT,
  completeModels,
  DEFAULT_SAMPLE,
  discoveryBlocker,
  formatRange,
  initialModels,
  MAX_SAMPLE,
  MIN_SAMPLE,
  rangeFromDates,
  sameModels,
  sampleSize,
  toDateInput,
} from "./discoveryForm";
import { formatCount } from "./profilesData";
import type { ModelChoice, ProfileSettingsView, TaxonomyView } from "./schemas";

type Role = keyof ModelChoice;

const ROLES: readonly { role: Role; label: string; hint: string; missing: string }[] = [
  {
    role: "discovery",
    label: "Task discovery model",
    hint: "Summarizes each sampled request, then names and describes the groups",
    missing: "No chat models with pricing and output limits are available from your gateway.",
  },
  {
    role: "embedding",
    label: "Embedding model",
    hint: "Places the summaries so similar requests group together",
    missing: "No embedding models are available. Add a gateway model reported with mode: embedding, then refresh.",
  },
  {
    role: "classification",
    label: "Classification model",
    hint: "Jev Choice with Noul assigns each new run to a reviewed task",
    missing: "No Jev models are available. Add a typesafe or decisions model to your gateway, then refresh.",
  },
];

const invalidateProfiles = (queryClient: ReturnType<typeof useQueryClient>) =>
  queryClient.invalidateQueries({ predicate: (entry) => String(entry.queryKey[1]).startsWith("profile") });

function ModelPickers({
  view,
  models,
  disabled,
  onChange,
}: {
  view: ProfileSettingsView;
  models: ModelChoice;
  disabled: boolean;
  onChange: (models: ModelChoice) => void;
}) {
  const baseId = useId();
  return (
    <div className="grid gap-4">
      {ROLES.map(({ role, label, hint, missing }) => {
        const options = view.options[role];
        const unavailable = models[role] !== "" && !options.includes(models[role]);
        return (
          <div key={role} className="space-y-1.5">
            <label htmlFor={`${baseId}-${role}`} className="text-xs font-medium">
              {label}
            </label>
            <SearchSelect
              inputId={`${baseId}-${role}`}
              aria-label={label}
              options={options.map((alias) => ({ value: alias, label: alias }))}
              value={models[role] || null}
              onValueChange={(value) => onChange({ ...models, [role]: value ?? "" })}
              placeholder={options.length ? "Choose a model" : "No models available"}
              emptyText={missing}
              disabled={disabled}
              allowClear
            />
            <p className="text-xs text-muted-foreground">{hint}</p>
            {options.length === 0 && <p className="text-xs text-amber-700 dark:text-amber-300">{missing}</p>}
            {unavailable && (
              <p role="alert" className="text-xs text-destructive">
                {models[role]} is not available on the gateway right now. Your selection is kept.
              </p>
            )}
          </div>
        );
      })}
    </div>
  );
}

function SettingsForm({
  agent,
  view,
  taxonomy,
  window,
  onStarted,
}: {
  agent: string;
  view: ProfileSettingsView;
  taxonomy: TaxonomyView | undefined;
  window: TimeWindow;
  onStarted: () => void;
}) {
  const api = useProfilesApi();
  const lens = useLensApi();
  const queryClient = useQueryClient();
  const switchId = useId();
  const helpId = useId();
  const labelId = useId();
  const startId = useId();
  const endId = useId();
  const sizeId = useId();
  const saved = view.settings;
  const [enabled, setEnabled] = useState(saved.enabled);
  const [models, setModels] = useState<ModelChoice>(() => initialModels(saved, view.defaults));
  const [start, setStart] = useState(() => toDateInput(window.startMs));
  const [end, setEnd] = useState(() => toDateInput(window.endMs - 1));
  const [size, setSize] = useState(String(DEFAULT_SAMPLE));
  const [confirming, setConfirming] = useState(false);
  const confirmId = useId();
  const discoverRef = useRef<HTMLButtonElement>(null);
  const closeConfirm = () => {
    setConfirming(false);
    discoverRef.current?.focus();
  };
  const escapeConfirm = (event: KeyboardEvent) => {
    if (event.key !== "Escape") return;
    event.stopPropagation();
    closeConfirm();
  };
  const [nowMs] = useState(Date.now);
  const dirty = enabled !== saved.enabled || !sameModels(models, saved.models);
  const range = rangeFromDates(start, end, nowMs);
  const runs = sampleSize(size);
  const write = view.can_write;
  const preview = useQuery({
    queryKey: ["lens", "profile-preview", api.scope, agent, range?.startMs, range?.endMs, runs],
    queryFn: ({ signal }) =>
      api.preview({ agent, window: range ?? window, maxRuns: runs ?? DEFAULT_SAMPLE }, signal),
    enabled: range !== null && runs !== null,
    staleTime: 30_000,
  });
  const save = useMutation({
    mutationFn: (next: { enabled: boolean; models: ModelChoice }) =>
      api.saveSettings({ agent, revision: saved.revision, ...next }),
    onSuccess: async (settings) => {
      setEnabled(settings.enabled);
      setModels(settings.models);
      queryClient.setQueryData<ProfileSettingsView>(["lens", "profile-settings", api.scope, agent], (current) =>
        current ? { ...current, settings } : current,
      );
      await invalidateProfiles(queryClient);
    },
  });
  const refresh = useMutation({
    mutationFn: () => lens.refreshGateway(),
    onSuccess: () => invalidateProfiles(queryClient),
  });
  const discover = useMutation({
    mutationFn: () =>
      api.discover({ agent, window: range ?? window, maxRuns: runs ?? DEFAULT_SAMPLE }, taxonomy?.taxonomy?.revision ?? 0),
    onSuccess: async () => {
      await invalidateProfiles(queryClient);
      onStarted();
    },
  });
  const blocker = discoveryBlocker({
    saved,
    dirty,
    range,
    size: runs,
    preview: preview.data,
    running: taxonomy?.taxonomy?.status === "discovering",
  });
  const enableBlocked = enabled && !completeModels(models);
  const busy = save.isPending || discover.isPending;
  return (
    <div className="flex flex-col gap-6 px-4 pb-6">
      <section aria-label="Opt in" className="space-y-3">
        <p role="status" className="inline-flex items-center gap-2 text-xs">
          <StatusDot state={saved.enabled ? "ok" : "off"} />
          {saved.enabled ? "Task discovery is on for this agent" : "Task discovery is off for this agent"}
        </p>
        <div className="flex items-start justify-between gap-4 rounded-lg border p-3">
          <div className="space-y-1">
            <label id={labelId} htmlFor={switchId} className="block text-sm font-medium">
              Use paid models for {agent}
            </label>
            <p id={helpId} className="text-xs text-muted-foreground">
              Saving never calls a model. Model calls only happen when you start a discovery or activate classification.
              Saving this off stops both and keeps existing results.
            </p>
          </div>
          <Switch
            id={switchId}
            aria-labelledby={labelId}
            aria-describedby={helpId}
            checked={enabled}
            onCheckedChange={(checked) => setEnabled(checked)}
            disabled={!write || busy}
          />
        </div>
        {enabled !== saved.enabled && (
          <p role="status" className="text-xs font-medium text-amber-700 dark:text-amber-300">
            Not saved yet. Task discovery stays {saved.enabled ? "on" : "off"} until you save settings below.
          </p>
        )}
        {write && saved.enabled && (
          <Button
            size="sm"
            variant="outline"
            disabled={busy}
            onClick={() => save.mutate({ enabled: false, models: saved.models })}
          >
            Turn off now
          </Button>
        )}
      </section>
      <section aria-label="Models" className="space-y-3">
        <div className="flex items-center justify-between gap-2">
          <h3 className="lens-section-label">Models</h3>
          {write && (
            <Button variant="outline" size="xs" disabled={refresh.isPending} onClick={() => refresh.mutate()}>
              <RefreshCw aria-hidden="true" className={`size-3 ${refresh.isPending ? "animate-spin" : ""}`} />
              {refresh.isPending ? "Refreshing…" : "Refresh models"}
            </Button>
          )}
        </div>
        <ModelPickers view={view} models={models} disabled={!write || busy} onChange={setModels} />
        {refresh.error && (
          <p role="alert" className="text-xs text-destructive">
            {refresh.error.message}
          </p>
        )}
        {write && (
          <div className="flex flex-wrap items-center gap-2">
            <Button size="sm" disabled={!dirty || enableBlocked || busy} onClick={() => save.mutate({ enabled, models })}>
              {save.isPending ? "Saving…" : "Save settings"}
            </Button>
            {enableBlocked && (
              <span className="text-xs text-muted-foreground">Choose all three models to turn discovery on.</span>
            )}
          </div>
        )}
        {save.error && (
          <p role="alert" className="text-sm text-destructive">
            {save.error.message}
          </p>
        )}
      </section>
      <section aria-label="Discover tasks" className="space-y-3">
        <h3 className="lens-section-label">Discover tasks</h3>
        <div className="grid gap-3 sm:grid-cols-3">
          <div className="space-y-1.5">
            <label htmlFor={startId} className="text-xs font-medium">
              From
            </label>
            <Input id={startId} type="date" value={start} max={end} onChange={(event) => setStart(event.target.value)} />
          </div>
          <div className="space-y-1.5">
            <label htmlFor={endId} className="text-xs font-medium">
              Through
            </label>
            <Input id={endId} type="date" value={end} min={start} onChange={(event) => setEnd(event.target.value)} />
          </div>
          <div className="space-y-1.5">
            <label htmlFor={sizeId} className="text-xs font-medium">
              Sample size
            </label>
            <Input
              id={sizeId}
              type="number"
              inputMode="numeric"
              min={MIN_SAMPLE}
              max={MAX_SAMPLE}
              value={size}
              onChange={(event) => setSize(event.target.value)}
            />
          </div>
        </div>
        <div role="status" aria-live="polite" className="space-y-2 rounded-lg border bg-muted/30 p-3 text-xs">
          {preview.isError && <p className="text-destructive">{preview.error.message}</p>}
          {preview.data && (
            <>
              <p>
                <span className="font-medium">{formatCount(preview.data.eligible_runs)}</span> eligible runs in this range.
                Discovery reads the most recent <span className="font-medium">{formatCount(preview.data.sample_runs)}</span>.
                Runs with a recorded category or no request are left out, and each run counts once.
              </p>
              {preview.data.previews.length > 0 && (
                <ul aria-label="Example requests" className="space-y-1">
                  {preview.data.previews.map((run, index) => (
                    <li key={`${index}:${run.trace_id}`} className="line-clamp-2 break-words text-muted-foreground">
                      {run.input_preview}
                    </li>
                  ))}
                </ul>
              )}
            </>
          )}
          {!preview.data && !preview.isError && (
            <p className="text-muted-foreground">
              {range && runs !== null ? BLOCKER_TEXT.previewing : "Choose a valid range and sample size to see eligible runs."}
            </p>
          )}
        </div>
        {write && blocker && blocker !== "previewing" && <p className="text-xs text-muted-foreground">{BLOCKER_TEXT[blocker]}</p>}
        {write && (
          <Button
            ref={discoverRef}
            size="sm"
            variant="outline"
            aria-expanded={confirming}
            aria-controls={confirmId}
            disabled={blocker !== null || busy}
            onClick={() => setConfirming(true)}
          >
            Discover tasks
          </Button>
        )}
        {confirming && range && runs !== null && preview.data && (
          <section
            id={confirmId}
            aria-label="Confirm discovery"
            className="space-y-3 rounded-lg border border-dashed p-3"
          >
            <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 text-xs">
              <dt className="text-muted-foreground">Agent</dt>
              <dd className="break-words">{agent}</dd>
              <dt className="text-muted-foreground">Range</dt>
              <dd>{formatRange(range.startMs, range.endMs)}</dd>
              <dt className="text-muted-foreground">Sample</dt>
              <dd>
                {formatCount(preview.data.sample_runs)} of {formatCount(preview.data.eligible_runs)} eligible runs
              </dd>
              <dt className="text-muted-foreground">Models</dt>
              <dd className="break-words">
                {saved.models.discovery}, {saved.models.embedding}, {saved.models.classification}
              </dd>
            </dl>
            <p className="text-xs">
              This makes paid calls through your gateway now. Your current reviewed tasks stay active until you activate the
              new draft.
            </p>
            <div className="flex gap-2">
              <Button
                size="sm"
                autoFocus
                onKeyDown={escapeConfirm}
                disabled={busy || blocker !== null}
                onClick={() => {
                  setConfirming(false);
                  discover.mutate();
                }}
              >
                Start discovery
              </Button>
              <Button size="sm" variant="ghost" onKeyDown={escapeConfirm} onClick={closeConfirm}>
                Cancel
              </Button>
            </div>
          </section>
        )}
        {discover.error && (
          <p role="alert" className="text-sm text-destructive">
            {discover.error.message}
          </p>
        )}
      </section>
    </div>
  );
}

export function DiscoverySettingsSheet({
  agent,
  window,
  open,
  onOpenChange,
}: {
  agent: string;
  window: TimeWindow;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const api = useProfilesApi();
  const settings = useQuery({
    queryKey: ["lens", "profile-settings", api.scope, agent],
    queryFn: ({ signal }) => api.settings(agent, signal),
    enabled: open && !!agent,
  });
  const taxonomy = useQuery({
    queryKey: ["lens", "profile-taxonomy", api.scope, agent],
    queryFn: ({ signal }) => api.taxonomy(agent, signal),
    enabled: open && !!agent,
  });
  const view = settings.data;
  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent className="w-full overflow-y-auto sm:max-w-lg">
        <SheetHeader>
          <SheetTitle>Task discovery</SheetTitle>
          <SheetDescription>
            {agent}. Opening and saving this panel makes no model calls.
          </SheetDescription>
        </SheetHeader>
        {settings.error && (
          <p role="alert" className="px-4 text-sm text-destructive">
            {settings.error.message}
          </p>
        )}
        {view && (
          <SettingsForm
            key={agent}
            agent={agent}
            view={view}
            taxonomy={taxonomy.data}
            window={window}
            onStarted={() => onOpenChange(false)}
          />
        )}
      </SheetContent>
    </Sheet>
  );
}
