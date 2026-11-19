"use client";

import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { StatusDot } from "../../shared/StatusDot";
import { Button } from "../../ui/button";
import { useProfilesApi } from "../data/LensServices";
import type { DiscoveryInput } from "./api";
import { classification, classificationText, formatDate, formatRange, MAX_SAMPLE } from "./discoveryForm";
import { formatCount } from "./profilesData";
import type { ModelChoice, ProfileSettings, ProfileSettingsView, TaskDefinition, Taxonomy, TaxonomyView } from "./schemas";

export type ProfileWrite =
  | { kind: "save"; models: ModelChoice }
  | { kind: "pause" }
  | { kind: "resume"; revision: number }
  | { kind: "activate"; revision: number; backfill: boolean }
  | { kind: "discard"; revision: number }
  | { kind: "discover"; input: DiscoveryInput; revision: number };

export const settingsKey = (scope: string, agent: string) => ["lens", "profile-settings", scope, agent];
export const taxonomyKey = (scope: string, agent: string) => ["lens", "profile-taxonomy", scope, agent];

const isProfileQuery = (key: readonly unknown[]) => String(key[1]).startsWith("profile");
const isLifecycleQuery = (key: readonly unknown[]) => key[1] === "profile-settings" || key[1] === "profile-taxonomy";

export function useProfileWrites(agent: string) {
  const api = useProfilesApi();
  const queryClient = useQueryClient();
  const settings = settingsKey(api.scope, agent);
  const taxonomy = taxonomyKey(api.scope, agent);
  const current = () => queryClient.getQueryData<ProfileSettingsView>(settings)?.settings;
  const storeSettings = async (saved: ProfileSettings) => {
    await queryClient.cancelQueries({ queryKey: settings });
    queryClient.setQueryData<ProfileSettingsView>(settings, (view) => (view ? { ...view, settings: saved } : view));
  };
  const storeTaxonomy = async (view: TaxonomyView) => {
    await queryClient.cancelQueries({ queryKey: taxonomy });
    queryClient.setQueryData<TaxonomyView>(taxonomy, view);
  };
  const save = async (enabled: boolean, models?: ModelChoice) => {
    const saved = current();
    if (!saved) throw new Error("Task discovery settings are not loaded");
    const update = { agent, revision: saved.revision, enabled, models: models ?? saved.models };
    await storeSettings(await api.saveSettings(update));
  };
  const enable = async () => {
    if (current()?.enabled === false) await save(true);
  };
  const run = async (write: ProfileWrite) => {
    switch (write.kind) {
      case "save":
        return save(true, write.models);
      case "pause":
        await save(false);
        return queryClient.invalidateQueries({ queryKey: taxonomy });
      case "resume":
        await enable();
        return storeTaxonomy(await api.resume(agent, write.revision));
      case "activate":
        await enable();
        return storeTaxonomy(await api.activate(agent, write.revision, write.backfill));
      case "discard":
        return storeTaxonomy(await api.reset(agent, write.revision, "draft"));
      case "discover":
        await enable();
        return storeTaxonomy(await api.discover(write.input, write.revision));
    }
  };
  return useMutation({
    mutationFn: run,
    onError: () =>
      Promise.all([
        queryClient.invalidateQueries({ queryKey: settings }),
        queryClient.invalidateQueries({ queryKey: taxonomy }),
      ]),
    onSettled: () =>
      queryClient.invalidateQueries({ predicate: ({ queryKey }) => isProfileQuery(queryKey) && !isLifecycleQuery(queryKey) }),
  });
}

function CategoryList({ tasks }: { tasks: readonly TaskDefinition[] }) {
  return (
    <ul aria-label="Categories" className="flex flex-col">
      {tasks.map((task) => (
        <li key={task.task_id} className="flex min-h-8 flex-col justify-center border-b border-border/60 py-1 text-xs last:border-0">
          <span className="font-medium">{task.name}</span>
          {task.description && <span className="line-clamp-1 text-muted-foreground">{task.description}</span>}
        </li>
      ))}
    </ul>
  );
}

function discoveredFrom(taxonomy: Taxonomy) {
  const sample = taxonomy.discovery ? `${formatCount(taxonomy.discovery.sampled_runs)} runs` : null;
  const date = taxonomy.discovered_at ? formatDate(taxonomy.discovered_at) : null;
  if (date && sample) return `Discovered ${date} from ${sample}`;
  if (sample) return `Discovered from ${sample}`;
  return date ? `Discovered ${date}` : null;
}

function ActiveCategories({
  active,
  write,
  busy,
  discovering,
  onWrite,
}: {
  active: Taxonomy;
  write: boolean;
  busy: boolean;
  discovering: boolean;
  onWrite: (write: ProfileWrite) => void;
}) {
  const state = classification(active);
  const source = discoveredFrom(active);
  return (
    <section aria-label="Active categories" className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="lens-section-label">Active categories</h3>
        {write && state.kind === "running" && (
          <Button size="xs" variant="outline" disabled={busy} onClick={() => onWrite({ kind: "pause" })}>
            Pause classification
          </Button>
        )}
        {write && state.kind === "paused" && (
          <Button size="xs" disabled={busy} onClick={() => onWrite({ kind: "resume", revision: active.revision })}>
            Resume classification
          </Button>
        )}
      </div>
      <p role="status" className="inline-flex items-center gap-2 text-xs">
        <StatusDot state={state.kind === "running" ? "ok" : "off"} />
        {classificationText(state)}
      </p>
      {source && <p className="text-xs text-muted-foreground">{source}</p>}
      {write && state.kind === "running" && discovering && (
        <p className="text-xs text-muted-foreground">Pausing also cancels the running discovery.</p>
      )}
      <CategoryList tasks={active.tasks} />
    </section>
  );
}

function DraftCategories({
  agent,
  draft,
  hasActive,
  write,
  busy,
  onWrite,
}: {
  agent: string;
  draft: Taxonomy;
  hasActive: boolean;
  write: boolean;
  busy: boolean;
  onWrite: (write: ProfileWrite) => void;
}) {
  const api = useProfilesApi();
  const [backfill, setBackfill] = useState(false);
  const range = draft.discovery;
  const backfillCount = useQuery({
    queryKey: ["lens", "profile-preview", api.scope, agent, range?.start_ms, range?.end_ms, MAX_SAMPLE],
    queryFn: ({ signal }) =>
      api.preview({ agent, window: { startMs: range?.start_ms ?? 0, endMs: range?.end_ms ?? 0 }, maxRuns: MAX_SAMPLE }, signal),
    enabled: draft.status === "draft" && !!range && write,
    staleTime: 30_000,
  });
  const discard = write && (
    <Button size="xs" variant="outline" disabled={busy} onClick={() => onWrite({ kind: "discard", revision: draft.revision })}>
      {draft.status === "discovering" ? "Cancel discovery" : "Discard draft"}
    </Button>
  );
  if (draft.status === "discovering") {
    return (
      <section aria-label="Discovery" className="flex flex-col gap-2">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <h3 className="lens-section-label">Discovering categories</h3>
          {discard}
        </div>
        <p role="status" className="inline-flex items-center gap-2 text-xs">
          <StatusDot state="warn" />
          {draft.job
            ? `Reading up to ${formatCount(draft.job.max_runs)} runs from ${formatRange(draft.job.start_ms, draft.job.end_ms)}.`
            : "Discovery is running."}
        </p>
        {hasActive && <p className="text-xs text-muted-foreground">The active categories keep classifying meanwhile.</p>}
      </section>
    );
  }
  if (draft.status === "failed") {
    return (
      <section aria-label="Discovery" className="flex flex-col gap-2">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <h3 className="lens-section-label">Discovery failed</h3>
          {discard}
        </div>
        <p role="alert" className="text-xs text-destructive">
          {draft.error ?? "Discovery failed"}
        </p>
      </section>
    );
  }
  const source = discoveredFrom(draft);
  return (
    <section aria-label="Discovered categories" className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="lens-section-label">
          {formatCount(draft.tasks.length)} discovered {draft.tasks.length === 1 ? "category" : "categories"}
        </h3>
        {discard}
      </div>
      {source && (
        <p className="text-xs text-muted-foreground">
          {source}
          {range && `, ${formatRange(range.start_ms, range.end_ms)}`}
        </p>
      )}
      <CategoryList tasks={draft.tasks} />
      {write && (
        <div className="flex flex-col gap-2 pt-1">
          {range && (
            <label className="flex items-start gap-2 text-xs">
              <input
                type="checkbox"
                className="mt-0.5"
                checked={backfill}
                onChange={(event) => setBackfill(event.target.checked)}
              />
              <span>
                Also classify earlier runs from {formatRange(range.start_ms, range.end_ms)}
                <span className="block text-muted-foreground">
                  {backfillCount.data
                    ? `At least ${formatCount(backfillCount.data.eligible_runs)} runs, one decision model call each.`
                    : "Counting runs in this range…"}
                </span>
              </span>
            </label>
          )}
          <p className="text-xs text-muted-foreground">
            {hasActive ? "Replaces the active categories. " : ""}
            {`Calls ${draft.models.decision} once for each new run.`}
          </p>
          <div>
            <Button
              size="sm"
              disabled={busy || draft.tasks.length === 0}
              onClick={() => onWrite({ kind: "activate", revision: draft.revision, backfill })}
            >
              Start classifying new runs
            </Button>
          </div>
        </div>
      )}
    </section>
  );
}

export function CategoryLifecycle({
  agent,
  view,
  busy,
  onWrite,
}: {
  agent: string;
  view: TaxonomyView;
  busy: boolean;
  onWrite: (write: ProfileWrite) => void;
}) {
  const draft = view.taxonomy;
  if (!view.active && !draft) return null;
  return (
    <>
      {draft && (
        <DraftCategories
          key={`${draft.taxonomy_id}:${draft.version}`}
          agent={agent}
          draft={draft}
          hasActive={!!view.active}
          write={view.can_write}
          busy={busy}
          onWrite={onWrite}
        />
      )}
      {view.active && (
        <ActiveCategories
          active={view.active}
          write={view.can_write}
          busy={busy}
          discovering={draft?.status === "discovering"}
          onWrite={onWrite}
        />
      )}
    </>
  );
}
