"use client";

import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../../ui/button";
import { useProfilesApi } from "../data/LensServices";
import type { TaxonomyTarget } from "./api";
import { classification, classificationText, formatRange } from "./discoveryForm";
import { formatCount } from "./profilesData";
import type { Capability, TaskDefinition, TaskEdit, Taxonomy, TaxonomyView } from "./schemas";

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

const TaskList = ({ tasks }: { tasks: readonly TaskDefinition[] }) => (
  <ul className="flex flex-wrap gap-2 text-sm">
    {tasks.map((task) => (
      <li key={task.task_id} className="rounded-md border px-2 py-1">
        {task.name}
      </li>
    ))}
  </ul>
);

function ActiveTasks({
  active,
  view,
  busy,
  onResume,
  onRemove,
}: {
  active: Taxonomy;
  view: TaxonomyView;
  busy: boolean;
  onResume: () => void;
  onRemove: () => void;
}) {
  const state = classification(active);
  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h4 className="text-sm font-medium">Active tasks</h4>
        {view.can_write && (
          <div className="flex flex-wrap gap-2">
            {state.kind === "paused" && (
              <Button size="sm" disabled={busy || !view.discovery_available} onClick={onResume}>
                Resume classification for new runs
              </Button>
            )}
            <Button size="sm" variant="outline" disabled={busy} onClick={onRemove}>
              Remove active tasks
            </Button>
          </div>
        )}
      </div>
      <p className="text-sm text-muted-foreground">{classificationText(state)}</p>
      {state.kind === "paused" && view.can_write && !view.discovery_available && (
        <p className="text-xs text-muted-foreground">Turn on task discovery for this agent to resume classification.</p>
      )}
      <TaskList tasks={active.tasks} />
    </div>
  );
}

function DraftTasks({
  agent,
  draft,
  view,
  hasActive,
  busy,
  onEdit,
  onActivate,
  onDiscard,
}: {
  agent: string;
  draft: Taxonomy;
  view: TaxonomyView;
  hasActive: boolean;
  busy: boolean;
  onEdit: (taskId: string, edit: TaskEdit) => void;
  onActivate: (backfill: boolean) => void;
  onDiscard: () => void;
}) {
  const api = useProfilesApi();
  const [backfill, setBackfill] = useState(false);
  const range = draft.discovery;
  const backfillCount = useQuery({
    queryKey: ["lens", "profile-preview", api.scope, agent, range?.start_ms, range?.end_ms, 500],
    queryFn: ({ signal }) =>
      api.preview({ agent, window: { startMs: range?.start_ms ?? 0, endMs: range?.end_ms ?? 0 }, maxRuns: 500 }, signal),
    enabled: draft.status === "draft" && !!range && view.can_write,
    staleTime: 30_000,
  });
  const write = view.can_write && !busy;
  return (
    <div className="space-y-3 rounded-lg border border-dashed p-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h4 className="text-sm font-medium">
          {draft.status === "discovering" ? "Discovering tasks" : draft.status === "failed" ? "Discovery failed" : "Draft to review"}
        </h4>
        {view.can_write && (
          <Button size="sm" variant="outline" disabled={!write} onClick={onDiscard}>
            {draft.status === "discovering" ? "Cancel discovery" : "Discard draft"}
          </Button>
        )}
      </div>
      {draft.status === "discovering" && (
        <p className="text-sm text-muted-foreground">
          Discovery is running{draft.job ? ` on up to ${formatCount(draft.job.max_runs)} runs from ${formatRange(draft.job.start_ms, draft.job.end_ms)}` : ""}.
          {hasActive ? " The active tasks keep classifying until you activate the new draft." : " The draft appears here when it finishes."}
        </p>
      )}
      {draft.status === "failed" && (
        <p role="alert" className="text-sm text-destructive">
          {draft.error ?? "Discovery failed"}
        </p>
      )}
      {draft.status === "draft" && range && (
        <p className="text-xs text-muted-foreground">
          Discovered from {formatCount(range.sampled_runs)} runs, {formatRange(range.start_ms, range.end_ms)}
          {`, with ${draft.models.distill}, ${draft.models.embedding} and ${draft.models.decision}`}.
        </p>
      )}
      {draft.status === "draft" && view.can_write && (
        <ul className="space-y-2">
          {draft.tasks.map((task) => (
            <TaskEditor
              key={`${draft.version}:${JSON.stringify(task)}`}
              task={task}
              saving={!write}
              onSave={(edit) => onEdit(task.task_id, edit)}
            />
          ))}
        </ul>
      )}
      {draft.status === "draft" && !view.can_write && <TaskList tasks={draft.tasks} />}
      {draft.status === "draft" && view.can_write && (
        <div className="space-y-2 border-t pt-3">
          {range && (
            <label className="flex items-start gap-2 text-sm">
              <input type="checkbox" className="mt-1" checked={backfill} onChange={(event) => setBackfill(event.target.checked)} />
              <span>
                Also classify earlier runs from {formatRange(range.start_ms, range.end_ms)}
                <span className="block text-xs text-muted-foreground">
                  {backfillCount.data
                    ? `At least ${formatCount(backfillCount.data.eligible_runs)} runs, one paid Jev call each.`
                    : "Counting runs in this range…"}
                </span>
              </span>
            </label>
          )}
          <p className="text-xs text-muted-foreground">
            {`Activating starts one paid ${draft.models.decision} call for each new run of this agent, until you turn task discovery off.`}
          </p>
          <Button size="sm" disabled={!write || !view.discovery_available} onClick={() => onActivate(backfill)}>
            Activate and classify new runs
          </Button>
          {!view.discovery_available && (
            <p className="text-xs text-muted-foreground">Turn on task discovery for this agent to activate a live draft.</p>
          )}
        </div>
      )}
    </div>
  );
}

export function TaxonomyPanel({ agent, onConfigure }: { agent: string; onConfigure: () => void }) {
  const api = useProfilesApi();
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: ["lens", "profile-taxonomy", api.scope, agent],
    queryFn: ({ signal }) => api.taxonomy(agent, signal),
    enabled: !!agent,
    refetchInterval: (current) => (current.state.data?.taxonomy?.status === "discovering" ? 10_000 : false),
  });
  const mutation = useMutation({
    mutationFn: (action: () => Promise<TaxonomyView>) => action(),
    onSuccess: () => queryClient.invalidateQueries({ predicate: (entry) => String(entry.queryKey[1]).startsWith("profile") }),
  });
  const view = query.data;
  if (!view || (!view.active && !view.taxonomy && !view.can_write)) return null;
  const busy = mutation.isPending;
  const reset = (taxonomy: Taxonomy, target: TaxonomyTarget) =>
    mutation.mutate(() => api.reset(agent, taxonomy.revision, target));
  return (
    <section aria-label="Reviewed tasks" className="space-y-4 rounded-xl border bg-card p-5">
      <h3 className="text-base font-medium">Reviewed tasks</h3>
      {!view.active && !view.taxonomy && (
        <div className="flex flex-wrap items-center justify-between gap-3">
          <p className="text-sm text-muted-foreground">
            Profiles come from recorded categories and free automatic grouping. Task discovery can propose reviewed tasks
            with paid models once you turn it on for this agent.
          </p>
          <Button size="sm" variant="outline" onClick={onConfigure}>
            Set up task discovery
          </Button>
        </div>
      )}
      {mutation.error && (
        <p role="alert" className="text-sm text-destructive">
          {mutation.error.message}
        </p>
      )}
      {view.active && (
        <ActiveTasks
          active={view.active}
          view={view}
          busy={busy}
          onResume={() => {
            const active = view.active;
            if (active) mutation.mutate(() => api.resume(agent, active.revision));
          }}
          onRemove={() => {
            if (view.active) reset(view.active, "active");
          }}
        />
      )}
      {view.taxonomy && (
        <DraftTasks
          key={`${view.taxonomy.taxonomy_id}:${view.taxonomy.version}`}
          agent={agent}
          draft={view.taxonomy}
          view={view}
          hasActive={!!view.active}
          busy={busy}
          onEdit={(taskId, edit) => {
            const draft = view.taxonomy;
            if (draft) mutation.mutate(() => api.editTask(agent, draft.revision, taskId, edit));
          }}
          onActivate={(backfill) => {
            const draft = view.taxonomy;
            if (draft) mutation.mutate(() => api.activate(agent, draft.revision, backfill));
          }}
          onDiscard={() => {
            if (view.taxonomy) reset(view.taxonomy, "draft");
          }}
        />
      )}
    </section>
  );
}
