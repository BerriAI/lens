"use client";

import { useState } from "react";
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
import type { InvestigationInput } from "../investigationSchema";
import type { ScopeOptions } from "../useMatchingActivity";

import { MetadataFilters } from "./MetadataFilters";
import { ProfileField } from "./ProfileField";

export function ScopeFields({ names, agentsLoading, agentsError, retryAgents, attributes, keys }: ScopeOptions) {
  const { control, register } = useFormContext<InvestigationInput>();
  const selection = useWatch({ control, name: "selection" });
  const filters = selection.filters ?? [];
  const hasOptionalScope = !!selection.team_id || !!selection.service;
  const showAdvancedByDefault = !!selection.filters.length || hasOptionalScope;
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
          Advanced filters{filters.length ? ` (${filters.length})` : ""}
        </summary>
        <div className="mt-4 space-y-4">
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
