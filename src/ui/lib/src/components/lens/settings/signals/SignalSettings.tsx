"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Flag, Trash2 } from "lucide-react";
import { useId, useState } from "react";
import { SetupAgentPrompt } from "../../onboarding/SetupAgentPrompt";
import { useLensHost } from "../../../../host/LensHost";

import { SearchSelect } from "../../../shared/SearchSelect";
import { StatusDot } from "../../../shared/StatusDot";
import { Button } from "../../../ui/button";
import { WatchPicker } from "../../setup/WatchPicker";
import { Input } from "../../../ui/input";
import { Skeleton } from "../../../ui/skeleton";
import { Textarea } from "../../../ui/textarea";

import { useLensApi } from "../../data/LensServices";
import { lensKeys, lensQueries } from "../../data/queries";
import { chatJudgeModels, SIGNAL_LIBRARY, signalsConfigured, systemOneModels } from "../../model/signals";
import type { SignalConfig } from "../../model/types";
import { configFrom, draftFrom, draftProblems, newRow, type SignalDraft, type SignalRow } from "./signalDraft";

import { SignalAlertSettings } from "./SignalAlertSettings";

const LIBRARY_QUESTIONS: ReadonlySet<string> = new Set(SIGNAL_LIBRARY.map((signal) => signal.question));

export function SignalSettings({
  section,
  onConfigureSignals,
}: {
  section: "signals" | "slack";
  onConfigureSignals: () => void;
}) {
  const api = useLensApi();
  const config = useQuery(lensQueries.signalConfig(api));
  if (config.error)
    return (
      <p role="alert" className="text-sm text-destructive">
        Could not load signals: {config.error.message}
      </p>
    );
  if (!config.data) return <Skeleton aria-label="Loading signals" className="h-40 w-full" />;
  return section === "signals" ? (
    <SignalForm saved={config.data} />
  ) : (
    <section aria-label="Slack delivery" className="space-y-4">
      <SignalAlertSettings signals={config.data} />
      <Button variant="link" size="sm" className="px-0" onClick={onConfigureSignals}>
        Configure signals
      </Button>
    </section>
  );
}

const FieldError = ({ children }: { children?: string }) =>
  children ? <p className="mt-1 text-xs text-destructive">{children}</p> : null;

function SetupCallout({ hasModels, gatewayConnected }: { hasModels: boolean; gatewayConnected: boolean }) {
  return (
    <div className="flex items-start gap-2 rounded-md bg-muted/40 p-3 text-sm">
      <Flag aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
      <div className="space-y-1">
        <p className="font-medium">Choose a model to start checking traces</p>
        {hasModels ? (
          <p className="text-xs text-muted-foreground">Pick one of the models configured for Lens below.</p>
        ) : (
          <p className="text-xs text-muted-foreground">
            {gatewayConnected
              ? "Your gateway has no evaluation or chat models. Add a model that supports the Decisions API or a chat model, then refresh models."
              : "Add an evaluation or analysis provider to your Lens deployment, then refresh models."}{" "}
            <a
              href="https://github.com/BerriAI/lens/blob/main/docs/signals.md"
              target="_blank"
              rel="noreferrer"
              className="font-medium text-foreground underline underline-offset-2"
            >
              Configure signals
            </a>
          </p>
        )}
      </div>
    </div>
  );
}

function SignalFields({
  row,
  problems,
  onChange,
  onRemove,
}: {
  row: SignalRow;
  problems?: { name?: string; question?: string };
  onChange: (row: SignalRow) => void;
  onRemove: () => void;
}) {
  const label = row.name.trim() || "new signal";
  return (
    <li className="grid gap-3 rounded-md border p-3 sm:grid-cols-[minmax(120px,1fr)_minmax(0,2fr)_auto]">
      <div>
        <Input
          aria-label="Signal name"
          placeholder="User frustration"
          value={row.name}
          aria-invalid={Boolean(problems?.name)}
          onChange={(event) => onChange({ ...row, name: event.target.value })}
        />
        <FieldError>{problems?.name}</FieldError>
      </div>
      <div>
        <Textarea
          aria-label={`Question for ${label}`}
          placeholder="Does the user show frustration with the agent in this run?"
          rows={2}
          value={row.question}
          aria-invalid={Boolean(problems?.question)}
          onChange={(event) => onChange({ ...row, question: event.target.value })}
        />
        <FieldError>{problems?.question}</FieldError>
      </div>
      <Button variant="ghost" size="icon-sm" aria-label={`Remove ${label}`} onClick={onRemove}>
        <Trash2 aria-hidden="true" />
      </Button>
    </li>
  );
}

export function SignalForm({ saved }: { saved: SignalConfig }) {
  const api = useLensApi();
  const standalone = useLensHost().surface === "standalone";
  const queryClient = useQueryClient();
  const modelId = useId();
  const thresholdId = useId();
  const [draft, setDraft] = useState<SignalDraft>(() => draftFrom(saved));
  const [draftBase, setDraftBase] = useState<SignalConfig>(() => saved);
  const [savedElsewhere, setSavedElsewhere] = useState(false);
  const savedKey = JSON.stringify(saved);
  const draftBaseKey = JSON.stringify(draftBase);
  if (savedKey !== draftBaseKey) {
    const draftIsDirty = JSON.stringify(configFrom(draft)) !== JSON.stringify(configFrom(draftFrom(draftBase)));
    setDraftBase(saved);
    if (draftIsDirty) {
      setSavedElsewhere(true);
    } else {
      setDraft(draftFrom(saved));
      setSavedElsewhere(false);
    }
  }
  const details = useQuery(lensQueries.modelDetails(api));
  const gateway = useQuery({
    ...lensQueries.gateway(api),
    enabled: standalone,
  });
  const discoveryError =
    details.error?.message ??
    (gateway.data?.configured && !gateway.data.connected
      ? (gateway.data.error ?? "The model gateway is not connected")
      : undefined);
  const systemOne = systemOneModels(details.data?.data ?? []);
  const judges = chatJudgeModels(details.data?.data ?? []);
  const models = [...systemOne, ...judges];
  const options = [
    ...systemOne.map((info) => ({
      value: info.model_group,
      label: info.model_group,
      sublabel: info.providers.join(", "),
    })),
    ...judges.map((info) => ({
      value: info.model_group,
      label: info.model_group,
      sublabel: ["Chat judge", ...info.providers].join(" · "),
    })),
  ];
  const problems = draftProblems(draft);
  const unavailable =
    Boolean(draft.model) && details.isSuccess && !models.some((info) => info.model_group === draft.model);
  const modelUnverified =
    Boolean(draft.model) && draft.model !== saved.model && !models.some((info) => info.model_group === draft.model);
  const next = configFrom(draft);
  const dirty = JSON.stringify(next) !== JSON.stringify(configFrom(draftFrom(draftBase)));
  const save = useMutation({
    mutationFn: (config: SignalConfig) => api.saveSignalConfig(config),
    onSuccess: (config) => {
      queryClient.setQueryData(lensKeys.signalConfig(api.scope), config);
      setDraft(draftFrom(config));
      setDraftBase(config);
      setSavedElsewhere(false);
      void queryClient.invalidateQueries({ queryKey: ["traceSignals"] });
      void queryClient.invalidateQueries({
        queryKey: lensKeys.signalOverview(api.scope),
      });
    },
  });
  const setRows = (rows: readonly SignalRow[]) => setDraft((current) => ({ ...current, rows }));
  const addRow = () => setRows([...draft.rows, newRow(crypto.randomUUID())]);
  const active = signalsConfigured(saved);
  const custom = draft.rows.filter((row) => !LIBRARY_QUESTIONS.has(row.question));
  const picked = new Set(
    SIGNAL_LIBRARY.filter((signal) => draft.rows.some((row) => row.question === signal.question)).map(
      (signal) => signal.id,
    ),
  );
  const pick = (next: ReadonlySet<string>) =>
    setRows([
      ...SIGNAL_LIBRARY.filter((signal) => next.has(signal.id)).map(
        (signal) =>
          draft.rows.find((row) => row.question === signal.question) ?? {
            key: crypto.randomUUID(),
            id: draft.rows.some((row) => row.id === signal.id) ? "" : signal.id,
            name: signal.name,
            question: signal.question,
          },
      ),
      ...custom,
    ]);
  const loadLatest = () => {
    setDraft(draftFrom(saved));
    setDraftBase(saved);
    setSavedElsewhere(false);
  };

  return (
    <div className="mx-auto w-full max-w-4xl space-y-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <p role="status" className="inline-flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
          <StatusDot state={active ? "ok" : "off"} />
          <span className="wrap-anywhere">
            {active ? `${saved.signals?.length} signals enabled` : "Signals are off"}
          </span>
        </p>
      </div>
      <section aria-label="Evaluation" className="space-y-4">
        {discoveryError ? (
          <div role="alert" className="flex flex-wrap items-center justify-between gap-3 rounded-md border p-3 text-sm">
            <p className="text-destructive">Could not load evaluation models: {discoveryError}</p>
            {details.isError ? (
              <Button
                variant="outline"
                size="sm"
                disabled={details.isFetching || gateway.isFetching}
                onClick={() => {
                  void details.refetch();
                }}
              >
                Retry model check
              </Button>
            ) : (
              <p className="text-xs text-muted-foreground">
                Your selection is kept. Refresh gateway models in Settings to try again.
              </p>
            )}
          </div>
        ) : !saved.model && !details.isPending ? (
          <SetupCallout hasModels={models.length > 0} gatewayConnected={Boolean(gateway.data?.connected)} />
        ) : null}
        {savedElsewhere && (
          <div role="alert" className="flex items-center justify-between gap-3 rounded-md border p-3 text-sm">
            <span>Signals were changed elsewhere</span>
            <Button type="button" variant="outline" size="sm" onClick={loadLatest}>
              Load latest
            </Button>
          </div>
        )}
        <div className="grid gap-4 sm:grid-cols-[minmax(0,1fr)_160px]">
          <div className="space-y-1.5">
            <label htmlFor={modelId} className="text-xs font-medium">
              Evaluation model
            </label>
            <SearchSelect
              inputId={modelId}
              aria-label="Evaluation model"
              options={options}
              value={draft.model}
              onValueChange={(value) => setDraft((current) => ({ ...current, model: value ?? "" }))}
              placeholder={details.isPending ? "Loading models…" : "Choose a model"}
              disabled={details.isPending}
              emptyText={discoveryError ? "Model list unavailable" : "No evaluation or chat models configured for Lens"}
              className="h-9 [&>[data-slot=input-group-addon]:has(>button)]:mr-0"
              allowClear
            />
            <p className="text-xs text-muted-foreground">
              Use an evaluation model or a chat model available to Lens. Chat models can handle longer traces, at a
              higher cost per trace.
            </p>
            {unavailable && !discoveryError && (
              <p className="text-xs text-muted-foreground">
                {draft.model} is not in the current model list. Your selection is kept.
              </p>
            )}
          </div>
          <div className="space-y-1.5">
            <label htmlFor={thresholdId} className="text-xs font-medium">
              Match threshold
            </label>
            <div className="flex items-center gap-1.5">
              <Input
                id={thresholdId}
                type="number"
                inputMode="numeric"
                min={5}
                max={95}
                step={5}
                value={Number.isNaN(draft.thresholdPercent) ? "" : draft.thresholdPercent}
                aria-invalid={Boolean(problems.threshold)}
                onChange={(event) =>
                  setDraft((current) => ({
                    ...current,
                    thresholdPercent: event.target.valueAsNumber,
                  }))
                }
              />
              <span className="text-sm text-muted-foreground">%</span>
            </div>
            <FieldError>{problems.threshold}</FieldError>
            <p className="text-xs leading-5 text-muted-foreground">Flag a trace at or above this score.</p>
          </div>
        </div>
        <p className="text-xs text-muted-foreground">
          Lens sends trace content to this model automatically. Your provider may charge for these requests.
        </p>
      </section>
      <div className="space-y-4 border-t pt-4">
        <WatchPicker
          label="Flag traces where"
          options={SIGNAL_LIBRARY}
          selected={picked}
          onChange={pick}
          onAddCustom={addRow}
        />
        {custom.length > 0 && (
          <ul aria-label="Custom signals" className="space-y-2">
            {custom.map((row) => (
              <SignalFields
                key={row.key}
                row={row}
                problems={problems.rows.get(row.key)}
                onChange={(changed) => setRows(draft.rows.map((other) => (other.key === row.key ? changed : other)))}
                onRemove={() => setRows(draft.rows.filter((other) => other.key !== row.key))}
              />
            ))}
          </ul>
        )}
        {draft.rows.length === 0 && (
          <p className="text-xs text-muted-foreground">Pick at least one signal to flag traces</p>
        )}
      </div>
      <div className="flex items-center justify-end gap-3 border-t border-border pt-4">
        {save.error && (
          <p role="alert" className="text-sm text-destructive">
            Could not save signals: {save.error.message}
          </p>
        )}
        {save.isSuccess && !dirty && <p className="text-xs text-muted-foreground">Saved</p>}
        <Button
          disabled={!dirty || problems.any || modelUnverified || save.isPending}
          onClick={() => save.mutate(next)}
        >
          {save.isPending ? "Saving…" : "Save signals"}
        </Button>
      </div>
      <details className="text-xs text-muted-foreground">
        <summary className="w-fit cursor-pointer rounded-sm outline-none focus-visible:ring-2 focus-visible:ring-ring">
          Set up with a coding agent
        </summary>
        <SetupAgentPrompt goal="signals" featureConfigured={active} />
      </details>
    </div>
  );
}
