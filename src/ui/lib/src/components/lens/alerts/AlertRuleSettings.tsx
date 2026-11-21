"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useId, useState } from "react";
import { Button } from "../../ui/button";
import { Checkbox } from "../../ui/checkbox";
import { Input } from "../../ui/input";
import { Skeleton } from "../../ui/skeleton";
import { useLensApi } from "../data/LensServices";
import type { AlertRuleConfig } from "../model/types";

export function AlertRuleSettings() {
  const api = useLensApi();
  const query = useQuery({
    queryKey: ["alert-rules", api.scope],
    queryFn: () => api.alertRules(),
    refetchOnWindowFocus: false,
  });
  return (
    <section aria-label="Alert rules" className="space-y-4">
      <div className="space-y-1">
        <h3 className="text-sm font-medium">Rules</h3>
        <p className="text-xs text-muted-foreground">Check timing and errors directly. No model needed.</p>
      </div>
      {query.error ? (
        <div role="alert" className="space-y-2 text-sm">
          <p>Could not load rules: {query.error.message}</p>
          <Button variant="outline" size="sm" onClick={() => void query.refetch()}>
            Retry
          </Button>
        </div>
      ) : query.data ? (
        <RuleForm key={query.data.revision} saved={query.data} />
      ) : (
        <Skeleton aria-label="Loading rules" className="h-32" />
      )}
    </section>
  );
}

function RuleForm({ saved }: { saved: AlertRuleConfig }) {
  const api = useLensApi();
  const queryClient = useQueryClient();
  const id = useId();
  const [durationEnabled, setDurationEnabled] = useState(saved.run_duration_minutes !== null);
  const [minutes, setMinutes] = useState(String(saved.run_duration_minutes ?? 5));
  const [modelErrors, setModelErrors] = useState(saved.model_errors);
  const valid =
    !durationEnabled || (/^\d+$/.test(minutes) && Number(minutes) >= 1 && Number(minutes) <= 4_294_967_295);
  const next = {
    revision: saved.revision,
    run_duration_minutes: durationEnabled ? Number(minutes) : null,
    model_errors: modelErrors,
  };
  const dirty =
    next.run_duration_minutes !== saved.run_duration_minutes || modelErrors !== saved.model_errors;
  const save = useMutation({
    mutationFn: () => api.saveAlertRules(next),
    onSuccess: (result) => queryClient.setQueryData(["alert-rules", api.scope], result),
  });
  return (
    <div className="space-y-4">
      <div className="divide-y border-y">
        <div className="flex flex-wrap items-center gap-x-3 gap-y-2 py-4">
          <Checkbox
            id={`${id}-duration`}
            checked={durationEnabled}
            onCheckedChange={setDurationEnabled}
            disabled={save.isPending}
          />
          <label htmlFor={`${id}-duration`} className="text-sm">
            Run exceeds
          </label>
          <Input
            aria-label="Run duration in minutes"
            aria-invalid={!valid}
            aria-describedby={!valid ? `${id}-error` : undefined}
            type="number"
            min={1}
            step={1}
            value={minutes}
            onChange={(event) => setMinutes(event.target.value)}
            disabled={!durationEnabled || save.isPending}
            className="h-8 w-20"
          />
          <span className="text-sm text-muted-foreground">minutes</span>
        </div>
        <label htmlFor={`${id}-model`} className="flex items-center gap-3 py-4 text-sm">
          <Checkbox
            id={`${id}-model`}
            checked={modelErrors}
            onCheckedChange={setModelErrors}
            disabled={save.isPending}
          />
          A model call fails
        </label>
      </div>
      {!valid && (
        <p id={`${id}-error`} role="alert" className="text-xs text-destructive">
          Enter a whole number of minutes, at least 1.
        </p>
      )}
      <p className="text-xs leading-5 text-muted-foreground">
        Applies to new runs across all agents and teams, once per rule and run. Checked as spans arrive. Runs
        that send telemetry only when finished are checked then.
      </p>
      <div className="flex flex-wrap items-center justify-between gap-3">
        <p className="text-xs text-muted-foreground">
          Appears in Alerts and uses your Slack notification settings.
        </p>
        <Button size="sm" disabled={!dirty || !valid || save.isPending} onClick={() => save.mutate()}>
          {save.isPending ? "Saving…" : "Save rules"}
        </Button>
      </div>
      {save.error && (
        <p role="alert" className="text-sm text-destructive">
          Could not save rules: {save.error.message}
        </p>
      )}
    </div>
  );
}
