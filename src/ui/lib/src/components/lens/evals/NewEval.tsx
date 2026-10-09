"use client";

import { useState, type ReactNode } from "react";
import { ChevronLeft } from "lucide-react";

import { Button } from "../../ui/button";
import { Checkbox } from "../../ui/checkbox";
import { Input } from "../../ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "../../ui/select";
import { Textarea } from "../../ui/textarea";
import { useDatasets } from "../datasets/api";
import { buildEvalSpec, EMPTY_DRAFT, type NewEvalDraft } from "./newEvalSpec";
import { useSaveEval } from "./runs/api";

export function NewEval({
  onCancel,
  onSaved,
  initialAgent = "",
}: {
  onCancel: () => void;
  onSaved: (name: string) => void;
  initialAgent?: string;
}) {
  const datasets = useDatasets();
  const save = useSaveEval();
  const [draft, setDraft] = useState<NewEvalDraft>({
    ...EMPTY_DRAFT,
    agent: initialAgent,
  });
  const [error, setError] = useState<string | null>(null);
  const set = (change: Partial<NewEvalDraft>) =>
    setDraft((current) => ({ ...current, ...change }));
  const items = (datasets.data ?? []).map((dataset) => ({
    value: dataset.id,
    label: `${dataset.name}@${dataset.revision}`,
  }));
  const submit = () => {
    const result = buildEvalSpec(draft);
    if (!result.ok) return setError(result.error);
    setError(null);
    save.mutate(result, {
      onSuccess: (saved) => onSaved(saved.name),
      onError: (failure) => setError(failure.message),
    });
  };
  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <header className="flex shrink-0 items-center justify-between gap-4 border-b px-6 py-5">
        <div>
          <h1 className="text-lg font-semibold tracking-tight">New eval</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Choose the dataset and scoring rules for your agent test
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={onCancel}>
          <ChevronLeft /> Evals
        </Button>
      </header>
      <form
        aria-label="New eval"
        className="min-h-0 flex-1 overflow-y-auto"
        onSubmit={(event) => {
          event.preventDefault();
          submit();
        }}
      >
        <div className="mx-auto my-5 grid max-w-3xl gap-4 px-4 text-sm">
          <p className="text-sm leading-6 text-muted-foreground">
            Saving an eval does not execute your agent. Next, connect this eval
            to your Python test or GitHub workflow to run its cases and see pass
            or fail results
          </p>
          <Section title="Eval">
            <Field label="Name">
              <Input
                className="font-mono"
                placeholder="agent-regressions"
                value={draft.name}
                onChange={(event) => set({ name: event.target.value })}
              />
            </Field>
            <Field label="Dataset">
              <Select
                items={items}
                value={draft.datasetId || null}
                onValueChange={(next: string | null) => {
                  const dataset = datasets.data?.find(
                    (item) => item.id === next,
                  );
                  set({
                    datasetId: next ?? "",
                    agent: draft.agent || dataset?.agent_name || "",
                  });
                }}
              >
                <SelectTrigger
                  aria-label="Dataset"
                  className="w-full font-mono text-xs"
                >
                  <SelectValue
                    placeholder={
                      datasets.isPending
                        ? "Loading datasets…"
                        : "Pick a dataset"
                    }
                  />
                </SelectTrigger>
                <SelectContent>
                  {items.map((item) => (
                    <SelectItem
                      key={item.value}
                      value={item.value}
                      className="font-mono text-xs"
                    >
                      {item.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </Field>
            <Field label="Agent">
              <Input
                readOnly={Boolean(initialAgent)}
                value={draft.agent}
                onChange={(event) => set({ agent: event.target.value })}
              />
            </Field>
          </Section>
          <Section title="Scorers">
            <Toggle
              label="Task completed"
              checked={draft.taskCompleted}
              onChange={(taskCompleted) => set({ taskCompleted })}
            />
            <Toggle
              label="Tool order"
              checked={draft.calledBefore}
              onChange={(calledBefore) => set({ calledBefore })}
            >
              <div className="flex items-center gap-2 font-mono text-xs">
                <Input
                  aria-label="First tool"
                  placeholder="run_tests"
                  value={draft.first}
                  onChange={(event) => set({ first: event.target.value })}
                />
                <span className="text-muted-foreground">before</span>
                <Input
                  aria-label="Then tool"
                  placeholder="open_pr"
                  value={draft.then}
                  onChange={(event) => set({ then: event.target.value })}
                />
              </div>
            </Toggle>
            <Toggle
              label="LLM judge"
              checked={draft.judge}
              onChange={(judge) => set({ judge })}
            >
              <Textarea
                aria-label="Judge question"
                placeholder="Did the agent complete the user's request?"
                value={draft.prompt}
                onChange={(event) => set({ prompt: event.target.value })}
              />
            </Toggle>
          </Section>
          <Section title="Pass criteria">
            <div className="grid grid-cols-2 gap-3 sm:grid-cols-5">
              <Field label="Trials">
                <Input
                  inputMode="numeric"
                  value={draft.trials}
                  onChange={(event) => set({ trials: event.target.value })}
                />
              </Field>
              <Field label="Max regressions">
                <Input
                  inputMode="numeric"
                  value={draft.regressions}
                  onChange={(event) => set({ regressions: event.target.value })}
                />
              </Field>
              <Field label="Max critical">
                <Input
                  inputMode="numeric"
                  value={draft.critical}
                  onChange={(event) => set({ critical: event.target.value })}
                />
              </Field>
              <Field label="Min pass %">
                <Input
                  inputMode="decimal"
                  placeholder="off"
                  value={draft.passRate}
                  onChange={(event) => set({ passRate: event.target.value })}
                />
              </Field>
              <Field label="Max $/case">
                <Input
                  inputMode="decimal"
                  placeholder="off"
                  value={draft.costPerCase}
                  onChange={(event) => set({ costPerCase: event.target.value })}
                />
              </Field>
            </div>
          </Section>
          <div className="lens-toolbar flex items-center justify-end gap-2 rounded-md border px-4 py-3">
            {error && (
              <p role="alert" className="mr-auto text-xs text-destructive">
                {error}
              </p>
            )}
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={onCancel}
            >
              Cancel
            </Button>
            <Button type="submit" size="sm" disabled={save.isPending}>
              Create eval
            </Button>
          </div>
        </div>
      </form>
    </div>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="grid gap-4 rounded-md border px-5 py-5 [&_input]:bg-background [&_textarea]:bg-background">
      <h3 className="text-sm font-medium">{title}</h3>
      {children}
    </section>
  );
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="grid gap-1 text-xs">
      <span className="font-mono text-[11px] text-muted-foreground">
        {label}
      </span>
      {children}
    </label>
  );
}

function Toggle({
  label,
  checked,
  onChange,
  children,
}: {
  label: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  children?: ReactNode;
}) {
  return (
    <div className="grid gap-3 rounded-lg border bg-background/60 p-3">
      <label className="flex items-center gap-2 text-xs">
        <Checkbox
          checked={checked}
          onCheckedChange={(next) => onChange(next === true)}
        />
        {label}
      </label>
      {checked && children && <div className="pl-6">{children}</div>}
    </div>
  );
}
