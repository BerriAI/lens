"use client";

import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { TimeWindow } from "../../shared/timeRange/timeRange";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import { useProfilesApi } from "../data/LensServices";
import type { Capability, TaskDefinition, TaskEdit, Taxonomy } from "./schemas";

const MAX_RUNS = 300;

const CAPABILITY_OPTIONS: readonly { value: Capability; label: string }[] = [
  { value: "function_calling", label: "Function calling" },
  { value: "vision", label: "Vision" },
  { value: "reasoning", label: "Reasoning" },
  { value: "structured_output", label: "Structured output" },
];

const inputClass = "w-full rounded-md border bg-background px-2 py-1";

function TaskEditor({ task, onSave, saving }: { task: TaskDefinition; onSave: (edit: TaskEdit) => void; saving: boolean }) {
  const [edit, setEdit] = useState<TaskEdit>({
    name: task.name,
    description: task.description,
    inclusion_criteria: task.inclusion_criteria,
    exclusion_criteria: task.exclusion_criteria,
    required_capabilities: task.required_capabilities,
  });
  const field = (key: "name" | "description" | "inclusion_criteria" | "exclusion_criteria", label: string, limit: number) => (
    <label className="grid gap-1">
      <span className="text-xs text-muted-foreground">{label}</span>
      <input
        className={inputClass}
        value={edit[key]}
        maxLength={limit}
        onChange={(event) => setEdit({ ...edit, [key]: event.target.value })}
      />
    </label>
  );
  const toggle = (capability: Capability) =>
    setEdit({
      ...edit,
      required_capabilities: edit.required_capabilities.includes(capability)
        ? edit.required_capabilities.filter((item) => item !== capability)
        : [...edit.required_capabilities, capability],
    });
  return (
    <li className="space-y-2 rounded-md border px-3 py-2 text-sm">
      {field("name", "Task name", 80)}
      {field("description", "Description", 600)}
      {field("inclusion_criteria", "Belongs when", 600)}
      {field("exclusion_criteria", "Does not belong", 600)}
      <fieldset className="grid gap-1">
        <legend className="text-xs text-muted-foreground">Every request needs</legend>
        <div className="flex flex-wrap gap-3">
          {CAPABILITY_OPTIONS.map((option) => (
            <label key={option.value} className="flex items-center gap-1.5">
              <input
                type="checkbox"
                checked={edit.required_capabilities.includes(option.value)}
                onChange={() => toggle(option.value)}
              />
              {option.label}
            </label>
          ))}
        </div>
      </fieldset>
      <Button size="sm" variant="outline" disabled={saving || !edit.name.trim()} onClick={() => onSave({ ...edit, name: edit.name.trim() })}>
        Save task
      </Button>
    </li>
  );
}

function statusText(taxonomy: Taxonomy | null): string {
  switch (taxonomy?.status) {
    case "discovering":
      return "Discovery is running. Draft tasks appear here when it finishes.";
    case "draft":
      return "Review the draft tasks, rename them if needed, then freeze to start classifying runs.";
    case "frozen":
      return "Runs are classified against these reviewed tasks. Recorded categories always take precedence.";
    case "failed":
      return `Discovery failed: ${taxonomy.error ?? "unknown error"}`;
    default:
      return "Profiles come from recorded categories and free automatic grouping.";
  }
}

export function TaxonomyPanel({ agent, window }: { agent: string; window: TimeWindow }) {
  const api = useProfilesApi();
  const queryClient = useQueryClient();
  const [confirming, setConfirming] = useState(false);
  const query = useQuery({
    queryKey: ["lens", "profile-taxonomy", api.scope, agent],
    queryFn: ({ signal }) => api.taxonomy(agent, signal),
    enabled: !!agent,
    refetchInterval: (current) => (current.state.data?.taxonomy?.status === "discovering" ? 10_000 : false),
  });
  const refresh = () =>
    queryClient.invalidateQueries({ predicate: (entry) => String(entry.queryKey[1]).startsWith("profile") });
  const mutation = useMutation({
    mutationFn: (action: () => Promise<Taxonomy>) => action(),
    onSuccess: refresh,
  });
  const view = query.data;
  const taxonomy = view?.taxonomy && view.taxonomy.status !== "cleared" ? view.taxonomy : null;
  if (!view || (!taxonomy && !view.discovery_available && !view.taxonomy?.sample)) return null;
  const write = view.can_write && !mutation.isPending;
  return (
    <section aria-label="Reviewed tasks" className="space-y-3 rounded-xl border bg-card p-5">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="flex items-center gap-2 text-base font-medium">
          Reviewed tasks
          {view.taxonomy?.sample && <Badge variant="secondary">Sample data</Badge>}
          {taxonomy && <Badge variant="outline">{taxonomy.status}</Badge>}
        </h3>
        {view.can_write && (
          <div className="flex flex-wrap gap-2">
            {taxonomy?.status === "draft" && (
              <Button size="sm" disabled={!write} onClick={() => mutation.mutate(() => api.freeze(agent, taxonomy.revision))}>
                {taxonomy.sample ? "Freeze sample (no model calls)" : "Freeze and classify"}
              </Button>
            )}
            {taxonomy && (
              <Button size="sm" variant="outline" disabled={!write} onClick={() => mutation.mutate(() => api.reset(agent, taxonomy.revision))}>
                {taxonomy.status === "discovering" ? "Cancel discovery" : "Reset"}
              </Button>
            )}
            {view.discovery_available && taxonomy?.status !== "discovering" && !confirming && (
              <Button size="sm" variant="outline" disabled={!write} onClick={() => setConfirming(true)}>
                Discover tasks
              </Button>
            )}
          </div>
        )}
      </div>
      <p className="text-sm text-muted-foreground">{statusText(taxonomy)}</p>
      {taxonomy?.status === "draft" && !taxonomy.sample && view.can_write && (
        <p className="text-sm text-muted-foreground">
          Freezing starts ongoing Jev classification calls for this agent&apos;s existing and new runs. Jev compares each run
          with the definitions below, and model compatibility uses the requirements you set.
        </p>
      )}
      {confirming && (
        <div role="alertdialog" aria-label="Confirm discovery" className="space-y-2 rounded-md border border-dashed p-3 text-sm">
          <p>
            Discovery calls your configured models on up to {MAX_RUNS} recent runs of {agent} in this range. This makes
            paid model calls through the gateway.
          </p>
          <div className="flex gap-2">
            <Button
              size="sm"
              disabled={!write}
              onClick={() => {
                setConfirming(false);
                mutation.mutate(() => api.discover({ agent, window, maxRuns: MAX_RUNS }));
              }}
            >
              Start discovery
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setConfirming(false)}>
              Cancel
            </Button>
          </div>
        </div>
      )}
      {mutation.error && (
        <p role="alert" className="text-sm text-destructive">
          {mutation.error.message}
        </p>
      )}
      {taxonomy?.status === "draft" && view.can_write && (
        <ul className="space-y-2">
          {taxonomy.tasks.map((task) => (
            <TaskEditor
              key={`${task.task_id}:${taxonomy.revision}`}
              task={task}
              saving={!write}
              onSave={(edit) => mutation.mutate(() => api.editTask(agent, taxonomy.revision, task.task_id, edit))}
            />
          ))}
        </ul>
      )}
      {taxonomy && (taxonomy.status === "frozen" || (taxonomy.status === "draft" && !view.can_write)) && (
        <ul className="flex flex-wrap gap-2 text-sm">
          {taxonomy.tasks.map((task) => (
            <li key={task.task_id} className="rounded-md border px-2 py-1">
              {task.name}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
