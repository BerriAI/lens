"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Plus, Trash2 } from "lucide-react";
import { useState } from "react";
import { Button } from "../../ui/button";
import { Checkbox } from "../../ui/checkbox";
import { Input } from "../../ui/input";
import { Skeleton } from "../../ui/skeleton";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "../../ui/dropdown-menu";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../ui/select";
import { useLensApi } from "../data/LensServices";
import type { AlertRuleConfig } from "../model/types";

type Rule = AlertRuleConfig["rules"][number];
type RuleDraft = Omit<Rule, "condition"> & {
  kind: Rule["condition"]["kind"];
  minutes: string;
};
const CONDITIONS = [
  { value: "run_duration", label: "Run duration" },
  { value: "model_error", label: "Model error" },
];
const validDuration = (rule: RuleDraft) =>
  rule.kind !== "run_duration" ||
  (/^\d+$/.test(rule.minutes) && Number(rule.minutes) >= 1 && Number(rule.minutes) <= 4_294_967_295);

export function AlertRuleSettings() {
  const api = useLensApi();
  const query = useQuery({
    queryKey: ["alert-rules", api.scope],
    queryFn: () => api.alertRules(),
    refetchOnWindowFocus: false,
  });
  return (
    <section aria-label="Alert rules" className="space-y-4">
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
  const [rules, setRules] = useState<RuleDraft[]>(() =>
    saved.rules.map((rule) => ({
      id: rule.id,
      enabled: rule.enabled,
      kind: rule.condition.kind,
      minutes: String(rule.condition.kind === "run_duration" ? rule.condition.minutes : 5),
    })),
  );
  const next: AlertRuleConfig = {
    revision: saved.revision,
    rules: rules.map((rule) => ({
      id: rule.id,
      enabled: rule.enabled,
      condition:
        rule.kind === "run_duration"
          ? { kind: "run_duration", minutes: Number(rule.minutes) }
          : { kind: "model_error" },
    })),
  };
  const dirty = JSON.stringify(next.rules) !== JSON.stringify(saved.rules);
  const save = useMutation({
    mutationFn: () => api.saveAlertRules(next),
    onSuccess: (result) => queryClient.setQueryData(["alert-rules", api.scope], result),
  });
  const update = (rule: RuleDraft) =>
    setRules((current) => current.map((item) => (item.id === rule.id ? rule : item)));
  const add = (kind: RuleDraft["kind"]) =>
    setRules((current) => [...current, { id: crypto.randomUUID(), enabled: true, kind, minutes: "5" }]);
  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between gap-3">
        <p className="text-xs text-muted-foreground">Check timing and errors. No model needed.</p>
        <DropdownMenu>
          <DropdownMenuTrigger
            render={<Button variant="outline" size="sm" disabled={save.isPending || rules.length >= 100} />}
          >
            <Plus aria-hidden="true" /> Add rule
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-48" aria-label="Choose a condition">
            <DropdownMenuItem onClick={() => add("run_duration")}>Run duration</DropdownMenuItem>
            <DropdownMenuItem onClick={() => add("model_error")}>Model error</DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
      {rules.length === 0 ? (
        <p className="rounded-md border border-dashed px-4 py-6 text-center text-sm text-muted-foreground">
          No rules yet. Add a condition to start checking new runs.
        </p>
      ) : (
        <div className="space-y-3">
          {rules.map((rule, index) => (
            <div
              key={rule.id}
              role="group"
              aria-label={`Rule ${index + 1}`}
              className="space-y-3 rounded-md border p-3"
            >
              <div className="flex items-center gap-3">
                <Select
                  value={rule.kind}
                  items={CONDITIONS}
                  disabled={save.isPending}
                  onValueChange={(kind) => {
                    if (kind === "run_duration" || kind === "model_error") update({ ...rule, kind });
                  }}
                >
                  <SelectTrigger aria-label="Condition" size="sm" className="min-w-0 flex-1">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {CONDITIONS.map((condition) => (
                      <SelectItem key={condition.value} value={condition.value}>
                        {condition.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <label className="flex items-center gap-2 text-xs">
                  <Checkbox
                    checked={rule.enabled}
                    onCheckedChange={(enabled) => update({ ...rule, enabled })}
                    disabled={save.isPending}
                  />
                  Enabled
                </label>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  aria-label={`Remove rule ${index + 1}`}
                  disabled={save.isPending}
                  onClick={() => setRules((current) => current.filter((item) => item.id !== rule.id))}
                >
                  <Trash2 aria-hidden="true" />
                </Button>
              </div>
              {rule.kind === "run_duration" ? (
                <div className="flex items-center gap-2 text-sm">
                  <span>Run exceeds</span>
                  <Input
                    aria-label="Run duration in minutes"
                    type="number"
                    min={1}
                    step={1}
                    value={rule.minutes}
                    aria-invalid={!validDuration(rule)}
                    aria-describedby={!validDuration(rule) ? `${rule.id}-error` : undefined}
                    onChange={(event) => update({ ...rule, minutes: event.target.value })}
                    disabled={save.isPending}
                    className="h-8 w-20"
                  />
                  <span className="text-muted-foreground">minutes</span>
                </div>
              ) : (
                <p className="text-sm text-muted-foreground">A model call fails</p>
              )}
              {!validDuration(rule) && (
                <p id={`${rule.id}-error`} role="alert" className="text-xs text-destructive">
                  Enter a whole number of minutes, at least 1.
                </p>
              )}
            </div>
          ))}
        </div>
      )}
      <div className="flex flex-wrap items-start justify-between gap-3">
        <p className="min-w-0 flex-1 basis-64 text-xs leading-5 text-muted-foreground">
          New runs only, once per rule and run. Checked as spans arrive, or after completion for integrations
          that export then. Uses your Slack settings.
        </p>
        <Button
          size="sm"
          disabled={!dirty || !rules.every(validDuration) || save.isPending}
          onClick={() => save.mutate()}
        >
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
