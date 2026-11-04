"use client";

import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Tags, X } from "lucide-react";
import { Button } from "../../../../ui/button";
import { Input } from "../../../../ui/input";
import { useTracesApi } from "../../api";
import { useOpenTraceRouting, useRunFilterRouting } from "../../routing";
import type { TraceSummary } from "../../types";
import type { TraceTag } from "../../tags";

export function TraceTags({
  summary,
  accessToken,
}: {
  summary: TraceSummary;
  accessToken: string;
}) {
  const api = useTracesApi(accessToken);
  const client = useQueryClient();
  const { setTags } = useRunFilterRouting();
  const { openTrace } = useOpenTraceRouting();
  const filterByTag = (tag: TraceTag) => {
    setTags([tag]);
    openTrace(null);
  };
  const [key, setKey] = useState("");
  const [value, setValue] = useState("");
  const queryKey = [
    "traceTags",
    accessToken,
    summary.trace_id,
    summary.trace_ref,
  ];
  const query = useQuery({
    queryKey,
    queryFn: () => api.tags!(summary.trace_id, summary.trace_ref),
    enabled: !!api.tags,
    retry: false,
  });
  const save = useMutation({
    mutationFn: (tags: readonly TraceTag[]) =>
      api.saveTags!(summary.trace_id, summary.trace_ref, tags),
    onSuccess: (tags) => {
      client.setQueryData(queryKey, tags);
      void client.invalidateQueries({ queryKey: ["agentTraces"] });
      void client.invalidateQueries({ queryKey: ["lens", "discovery"] });
      void client.invalidateQueries({ queryKey: ["lens", "preview"] });
      setKey("");
      setValue("");
    },
  });
  if (!api.tags) return null;
  if (query.error)
    return (
      <section
        aria-label="Trace tags"
        className="flex items-center justify-between gap-2 border-b px-4 py-2 text-xs"
      >
        <p role="alert" className="text-muted-foreground">
          Tags are unavailable. Try loading them again.
        </p>
        <Button
          variant="ghost"
          size="xs"
          disabled={query.isFetching}
          onClick={() => void query.refetch()}
        >
          Retry tags
        </Button>
      </section>
    );
  if (!query.data) return null;
  const { recorded, manual, editable } = query.data;
  const canEdit = editable && !!api.saveTags;
  const add = () => {
    const tag = { key: key.trim(), value: value.trim() };
    if (!tag.key || !tag.value) return;
    save.mutate([...manual.filter((item) => item.key !== tag.key), tag]);
  };
  return (
    <section
      aria-label="Trace tags"
      className="flex shrink-0 flex-col gap-3 border-b px-4 py-3"
    >
      <div className="flex items-center gap-2 text-xs font-medium">
        <Tags className="size-3.5" /> Tags
      </div>
      <div className="flex flex-wrap gap-1.5">
        {manual.map((tag) => (
          <span
            key={tag.key}
            className="inline-flex items-center rounded-md border bg-primary/5"
          >
            <Button variant="ghost" size="xs" onClick={() => filterByTag(tag)}>
              {tag.key}: {tag.value}
            </Button>
            {canEdit && (
              <Button
                variant="ghost"
                size="icon-xs"
                disabled={save.isPending}
                aria-label={`Remove tag ${tag.key}: ${tag.value}`}
                onClick={() =>
                  save.mutate(manual.filter((item) => item.key !== tag.key))
                }
              >
                <X className="size-3" />
              </Button>
            )}
          </span>
        ))}
        {!recorded.length && !manual.length && (
          <p className="text-xs text-muted-foreground">No tags yet</p>
        )}
      </div>
      {recorded.length > 0 && (
        <details className="text-xs">
          <summary className="cursor-pointer text-muted-foreground">
            Recorded tags ({recorded.length})
          </summary>
          <div className="mt-2 flex flex-wrap gap-1.5">
            {recorded.map((tag) => (
              <Button
                key={`recorded:${tag.key}:${tag.value}`}
                size="xs"
                variant="outline"
                title="Recorded metadata, click to filter traces"
                onClick={() => filterByTag(tag)}
              >
                {tag.key}: {tag.value}
              </Button>
            ))}
          </div>
        </details>
      )}
      {canEdit && (
        <div className="flex flex-wrap items-end gap-2">
          <label className="grid gap-1 text-xs">
            Tag key
            <Input
              className="h-8 w-36"
              maxLength={256}
              value={key}
              onChange={(event) => setKey(event.target.value)}
              placeholder="skill"
            />
          </label>
          <label className="grid gap-1 text-xs">
            Tag value
            <Input
              className="h-8 w-36"
              maxLength={256}
              value={value}
              onChange={(event) => setValue(event.target.value)}
              placeholder="diagram"
            />
          </label>
          <Button
            size="sm"
            className="h-8"
            disabled={
              !key.trim() ||
              !value.trim() ||
              save.isPending ||
              (manual.length >= 8 &&
                !manual.some((tag) => tag.key === key.trim()))
            }
            onClick={add}
          >
            {save.isPending ? "Saving…" : "Save tag"}
          </Button>
        </div>
      )}
      {save.error && (
        <p role="alert" className="text-xs text-destructive">
          {save.error.message}
        </p>
      )}
    </section>
  );
}
