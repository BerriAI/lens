"use client";

import { useId, useRef, useState, type KeyboardEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { RefreshCw } from "lucide-react";
import { SearchSelect } from "../../shared/SearchSelect";
import type { TimeWindow } from "../../shared/timeRange/timeRange";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "../../ui/sheet";
import { useLensApi, useProfilesApi } from "../data/LensServices";
import { CategoryLifecycle, settingsKey, taxonomyKey, useProfileWrites, type ProfileWrite } from "./CategoryLifecycle";
import {
  blockerText,
  completeModels,
  DEFAULT_SAMPLE,
  discoveryBlocker,
  eligibilityText,
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
    label: "Discovery model",
    hint: "Summarizes sampled requests and names the categories",
    missing: "No chat models with pricing and output limits are available from your gateway.",
  },
  {
    role: "embedding",
    label: "Embedding model",
    hint: "Groups similar requests together",
    missing: "No embedding models are available from your gateway.",
  },
  {
    role: "classification",
    label: "Decision model",
    hint: "Assigns each new run to a category",
    missing: "No decision models are available from your gateway.",
  },
];

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
    <div className="flex flex-col gap-3">
      {ROLES.map(({ role, label, hint, missing }) => {
        const options = view.options[role];
        const unavailable = models[role] !== "" && !options.includes(models[role]);
        return (
          <div key={role} className="grid gap-1 sm:grid-cols-[9rem_minmax(0,1fr)] sm:items-start sm:gap-x-3">
            <label htmlFor={`${baseId}-${role}`} className="pt-1.5 text-xs font-medium">
              {label}
            </label>
            <div className="flex min-w-0 flex-col gap-1">
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
              <p className="text-xs text-muted-foreground">{options.length === 0 ? missing : hint}</p>
              {unavailable && (
                <p role="alert" className="text-xs text-destructive">
                  {models[role]} is not available on the gateway right now. Your selection is kept.
                </p>
              )}
            </div>
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
}: {
  agent: string;
  view: ProfileSettingsView;
  taxonomy: TaxonomyView | undefined;
  window: TimeWindow;
}) {
  const api = useProfilesApi();
  const lens = useLensApi();
  const queryClient = useQueryClient();
  const startId = useId();
  const endId = useId();
  const sizeId = useId();
  const confirmId = useId();
  const saved = view.settings;
  const [models, setModels] = useState<ModelChoice>(() => initialModels(saved, view.defaults));
  const [start, setStart] = useState(() => toDateInput(window.startMs));
  const [end, setEnd] = useState(() => toDateInput(window.endMs - 1));
  const [size, setSize] = useState(String(DEFAULT_SAMPLE));
  const [confirming, setConfirming] = useState(false);
  const discoverRef = useRef<HTMLButtonElement>(null);
  const [nowMs, setNowMs] = useState(Date.now);
  const writes = useProfileWrites(agent);
  const dirty = !sameModels(models, saved.models);
  const range = rangeFromDates(start, end, nowMs);
  const runs = sampleSize(size);
  const write = view.can_write;
  const busy = writes.isPending;
  const preview = useQuery({
    queryKey: ["lens", "profile-preview", api.scope, agent, range?.startMs, range?.endMs, runs],
    queryFn: ({ signal }) => api.preview({ agent, window: range ?? window, maxRuns: runs ?? DEFAULT_SAMPLE }, signal),
    enabled: range !== null && runs !== null,
    staleTime: 30_000,
  });
  const refresh = useMutation({
    mutationFn: () => lens.refreshGateway(),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: settingsKey(api.scope, agent) }),
  });
  const refreshCount = () => {
    const latest = Date.now();
    const next = rangeFromDates(start, end, latest);
    if (next && range && next.endMs !== range.endMs) setNowMs(latest);
    else void preview.refetch();
  };
  const closeConfirm = () => {
    setConfirming(false);
    discoverRef.current?.focus();
  };
  const escapeConfirm = (event: KeyboardEvent) => {
    if (event.key !== "Escape") return;
    event.stopPropagation();
    closeConfirm();
  };
  const perform = (next: ProfileWrite) => writes.mutate(next);
  const draft = taxonomy?.taxonomy ?? null;
  const discovering = draft?.status === "discovering";
  const blocker = discoveryBlocker({ saved, dirty, range, size: runs, preview: preview.data, running: discovering });
  const rediscover = !!taxonomy?.active || !!draft;
  const action = rediscover ? "Rediscover categories" : "Discover categories";
  const eligibility = preview.isError
    ? preview.error.message
    : eligibilityText(preview.data, range !== null && runs !== null);
  return (
    <div className="flex flex-col gap-6 px-4 pb-6">
      <section aria-label="Models" className="flex flex-col gap-3">
        <div className="flex items-center justify-between gap-2">
          <h3 className="lens-section-label">Models</h3>
          {write && (
            <Button variant="ghost" size="xs" disabled={refresh.isPending} onClick={() => refresh.mutate()}>
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
            <Button
              size="sm"
              variant="outline"
              disabled={!dirty || !completeModels(models) || busy}
              onClick={() =>
                writes.mutate(
                  { kind: "save", models },
                  {
                    onSuccess: () =>
                      setModels(
                        queryClient.getQueryData<ProfileSettingsView>(settingsKey(api.scope, agent))?.settings.models ?? models,
                      ),
                  },
                )
              }
            >
              Save models
            </Button>
            <span role="status" className="text-xs text-muted-foreground">
              {!completeModels(models)
                ? "Choose all three models."
                : dirty
                  ? "Not saved yet. Saving does not call any model."
                  : "Saved"}
            </span>
          </div>
        )}
      </section>
      {writes.error && (
        <p role="alert" className="text-xs text-destructive">
          {writes.error.message}
        </p>
      )}
      {taxonomy && <CategoryLifecycle agent={agent} view={taxonomy} busy={busy} onWrite={perform} />}
      {!discovering && (
        <section aria-label={action} className="flex flex-col gap-3">
          <h3 className="lens-section-label">{action}</h3>
          <div className="grid gap-3 sm:grid-cols-3">
            <div className="flex flex-col gap-1">
              <label htmlFor={startId} className="text-xs font-medium">
                From
              </label>
              <Input id={startId} type="date" value={start} max={end} onChange={(event) => setStart(event.target.value)} />
            </div>
            <div className="flex flex-col gap-1">
              <label htmlFor={endId} className="text-xs font-medium">
                Through
              </label>
              <Input id={endId} type="date" value={end} min={start} onChange={(event) => setEnd(event.target.value)} />
            </div>
            <div className="flex flex-col gap-1">
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
          <div className="flex flex-wrap items-center justify-between gap-2 text-xs">
            <p role="status" aria-live="polite" className={preview.isError ? "text-destructive" : undefined}>
              {eligibility}
            </p>
            <Button
              variant="ghost"
              size="xs"
              disabled={preview.isFetching || range === null || runs === null}
              onClick={refreshCount}
            >
              <RefreshCw aria-hidden="true" className={`size-3 ${preview.isFetching ? "animate-spin" : ""}`} />
              Refresh count
            </Button>
          </div>
          {write && blocker && blocker !== "previewing" && blocker !== "too-few" && (
            <p className="text-xs text-muted-foreground">{blockerText(blocker, preview.data)}</p>
          )}
          {write && (
            <div>
              <Button
                ref={discoverRef}
                size="sm"
                variant={rediscover ? "outline" : "default"}
                aria-expanded={confirming}
                aria-controls={confirmId}
                disabled={blocker !== null || busy}
                onClick={() => setConfirming(true)}
              >
                {action}
              </Button>
            </div>
          )}
          {confirming && range && runs !== null && preview.data && (
            <section id={confirmId} aria-label="Confirm discovery" className="flex flex-col gap-3 border-l-2 pl-3">
              <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 text-xs">
                <dt className="text-muted-foreground">Range</dt>
                <dd>{formatRange(range.startMs, range.endMs)}</dd>
                <dt className="text-muted-foreground">Sample</dt>
                <dd>
                  {formatCount(preview.data.sample_runs)} of {formatCount(preview.data.eligible_runs)} eligible runs
                </dd>
                <dt className="text-muted-foreground">Models</dt>
                <dd className="break-words font-mono">
                  {saved.models.discovery}, {saved.models.embedding}, {saved.models.classification}
                </dd>
              </dl>
              <p className="text-xs text-muted-foreground">
                Calls these models through your gateway for each sampled run.
                {taxonomy?.active ? " The active categories keep classifying until you start the new ones." : ""}
              </p>
              <div className="flex gap-2">
                <Button
                  size="sm"
                  autoFocus
                  onKeyDown={escapeConfirm}
                  disabled={busy || blocker !== null}
                  onClick={() => {
                    setConfirming(false);
                    perform({
                      kind: "discover",
                      input: { agent, window: range, maxRuns: runs },
                      revision: draft?.revision ?? 0,
                    });
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
        </section>
      )}
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
    queryKey: settingsKey(api.scope, agent),
    queryFn: ({ signal }) => api.settings(agent, signal),
    enabled: open && !!agent,
  });
  const taxonomy = useQuery({
    queryKey: taxonomyKey(api.scope, agent),
    queryFn: ({ signal }) => api.taxonomy(agent, signal),
    enabled: open && !!agent,
    refetchInterval: (query) => (query.state.data?.taxonomy?.status === "discovering" ? 10_000 : false),
  });
  const view = settings.data;
  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent className="w-full overflow-y-auto sm:max-w-lg">
        <SheetHeader>
          <SheetTitle>Task discovery</SheetTitle>
          <SheetDescription>{agent}</SheetDescription>
        </SheetHeader>
        {settings.error && (
          <p role="alert" className="px-4 text-sm text-destructive">
            {settings.error.message}
          </p>
        )}
        {taxonomy.error && (
          <p role="alert" className="px-4 text-sm text-destructive">
            {taxonomy.error.message}
          </p>
        )}
        {view && <SettingsForm key={agent} agent={agent} view={view} taxonomy={taxonomy.data} window={window} />}
      </SheetContent>
    </Sheet>
  );
}
