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
import { SearchSelect } from "../../../shared/SearchSelect";
import { useLensApi } from "../../data/LensServices";
import { lensQueries } from "../../data/queries";
import { signalsConfigured } from "../../model/signals";
import type { InvestigationInput } from "../investigationSchema";
import type { ScopeOptions } from "../useMatchingActivity";

import { MetadataFilters } from "./MetadataFilters";
import { ProfileField } from "./ProfileField";

export function ScopeFields({ names, agentsLoading, agentsError, retryAgents, attributes, keys }: ScopeOptions) {
  const { control, register } = useFormContext<InvestigationInput>();
  const api = useLensApi();
  const signals = useQuery(lensQueries.signalConfig(api));
  const selection = useWatch({ control, name: "selection" });
  const configuredSignals = signals.data && signalsConfigured(signals.data) ? signals.data.signals : [];
  const selectedSignal = configuredSignals.find((signal) => signal.id === selection.signal_id);
  const signalItems = configuredSignals.map((signal) => ({ value: signal.id, label: signal.name }));
  const filters = selection.filters ?? [];
  const filterCount = filters.length + Number(!!selection.signal_id);
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
          Advanced filters{filterCount ? ` (${filterCount})` : ""}
        </summary>
        <div className="mt-4 space-y-4">
          <label className="grid gap-2 text-sm font-medium">
            Signal result (optional)
            <Controller
              control={control}
              name="selection.signal_id"
              render={({ field }) => (
                <SearchSelect
                  aria-label="Signal result (optional)"
                  options={signalItems}
                  value={field.value}
                  onValueChange={field.onChange}
                  placeholder="All runs"
                  emptyText={signals.isPending ? "Loading signals…" : "No matching signals"}
                  className="h-9"
                />
              )}
            />
            <span className="text-xs font-normal text-muted-foreground">
              {signals.isPending
                ? "Loading signals from Alerts…"
                : signals.error
                  ? "Could not load signals from Alerts. Your selected filter is preserved."
                  : !configuredSignals.length
                    ? "Configure model signals in Alerts to filter runs by their saved results."
                    : selection.signal_id && !selectedSignal
                      ? "This signal is no longer configured in Alerts. Choose another signal or clear the filter."
                      : selection.signal_id
                        ? "Uses saved results for signals configured in Alerts, at the current alert threshold."
                        : "Leave empty to include all runs in your selected scope."}
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
