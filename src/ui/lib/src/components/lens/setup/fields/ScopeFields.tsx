"use client";

import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Controller, useFormContext, useWatch } from "react-hook-form";
import {
  Combobox,
  ComboboxInput,
  ComboboxContent,
  ComboboxList,
  ComboboxItem,
  ComboboxEmpty,
} from "../../../ui/combobox";
import { Input } from "../../../ui/input";
import { Badge } from "../../../ui/badge";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../../ui/select";
import { useLensApi } from "../../data/LensServices";
import { lensQueries } from "../../data/queries";
import type { InvestigationInput } from "../investigationSchema";
import type { ScopeOptions } from "../useMatchingActivity";

import { MetadataFilters } from "./MetadataFilters";
import { ProfileField } from "./ProfileField";

export function ScopeFields({ names, agentsLoading, agentsError, retryAgents, attributes, keys }: ScopeOptions) {
  const { control, register, setValue } = useFormContext<InvestigationInput>();
  const api = useLensApi();
  const signals = useQuery(lensQueries.signalConfig(api));
  const selection = useWatch({ control, name: "selection" });
  const configuredSignals = signals.data?.signals ?? [];
  const selectedSignal = configuredSignals.find((signal) => signal.id === selection.signal_id);
  const signalItems = [
    { value: "", label: "All runs" },
    ...configuredSignals.map((signal) => ({ value: signal.id, label: signal.name })),
    ...(selection.signal_id && !selectedSignal
      ? [{ value: selection.signal_id, label: `${selection.signal_id} (unavailable)` }]
      : []),
  ];
  const filters = selection.filters ?? [];
  const hasOptionalScope = !!selection.team_id || !!selection.service;
  const showAdvancedByDefault = !!selection.filters.length || hasOptionalScope || !!selection.signal_id;
  const [advanced, setAdvanced] = useState(showAdvancedByDefault);
  const nameLabel = "Agent (optional)";
  return (
    <>
      <label className="grid gap-2 text-sm font-medium">
        {nameLabel}
        <Controller
          control={control}
          name="selection.agent_name"
          render={({ field }) => (
            <Combobox
              items={names}
              value={field.value || null}
              inputValue={field.value ?? ""}
              onInputValueChange={field.onChange}
              onValueChange={(name) => field.onChange(name ?? "")}
            >
              <ComboboxInput
                aria-label={nameLabel}
                placeholder="All agents and activity"
                showClear={!!selection.agent_name}
                className="w-full h-9"
              />
              <ComboboxContent>
                <ComboboxEmpty>
                  {agentsLoading ? "Loading agents…" : "No matches. You can enter a recorded name."}
                </ComboboxEmpty>
                <ComboboxList>
                  {(name: string) => (
                    <ComboboxItem key={name} value={name}>
                      {name}
                    </ComboboxItem>
                  )}
                </ComboboxList>
              </ComboboxContent>
            </Combobox>
          )}
        />
      </label>
      <ProfileField />
      {selection.signal_id && (
        <Badge variant="secondary" className="w-fit gap-2">
          Signal: {selectedSignal?.name ?? selection.signal_id}
          <button
            type="button"
            aria-label="Clear signal filter"
            onClick={() => setValue("selection.signal_id", null, { shouldDirty: true })}
          >
            ×
          </button>
        </Badge>
      )}
      {agentsError && (
        <p role="alert" className="text-sm text-destructive">
          Could not load agents.{" "}
          <button type="button" className="underline" onClick={retryAgents}>
            Retry
          </button>
        </p>
      )}
      <details open={advanced} onToggle={(event) => setAdvanced(event.currentTarget.open)} className="group">
        <summary className="cursor-pointer text-sm font-medium">
          Advanced filters{filters.length ? ` (${filters.length})` : ""}
        </summary>
        <div className="mt-4 space-y-4">
          <label className="grid gap-2 text-sm font-medium">
            Signal result
            <Controller
              control={control}
              name="selection.signal_id"
              render={({ field }) => (
                <Select
                  items={signalItems}
                  value={field.value ?? ""}
                  onValueChange={(value: string | null) => field.onChange(value || null)}
                >
                  <SelectTrigger aria-label="Signal result" className="w-full bg-background">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {signalItems.map((item) => (
                      <SelectItem key={item.value} value={item.value}>{item.label}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              )}
            />
            <span className="text-xs font-normal text-muted-foreground">
              {signals.isPending
                ? "Loading signals from Alerts…"
                : signals.error
                  ? "Could not load signals from Alerts. Your selected filter is preserved."
                  : selection.signal_id && !selectedSignal
                    ? "This signal is no longer configured in Alerts. Choose another signal or all runs."
                    : configuredSignals.length
                      ? "Uses saved results for signals configured in Alerts, at the current alert threshold."
                      : "Configure model signals in Alerts to filter runs by their saved results."}
            </span>
          </label>
          {signals.error && (
            <button type="button" className="text-sm underline" onClick={() => void signals.refetch()}>
              Retry loading signals
            </button>
          )}
          <label className="grid gap-2 text-sm">
            Application (optional)
            <Input {...register("selection.service")} placeholder="All applications" />
          </label>
          <p className="text-xs leading-5 text-muted-foreground">
            Match any recorded metadata, such as a user ID, environment, or tag. All conditions must match.
          </p>
          <MetadataFilters attributes={attributes} keys={keys} />
          <label className="grid gap-2 text-sm">
            Team ID (optional)
            <Input {...register("selection.team_id")} placeholder="All accessible teams" />
          </label>
        </div>
      </details>
    </>
  );
}
