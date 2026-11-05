"use client";

import { useState } from "react";
import { skipToken, useQuery } from "@tanstack/react-query";
import { Tags, X } from "lucide-react";
import { Button } from "../../../../ui/button";
import { Input } from "../../../../ui/input";
import {
  Popover,
  PopoverContent,
  PopoverTitle,
  PopoverTrigger,
} from "../../../../ui/popover";
import { useRunFilterRouting } from "../../routing";
import { useDialogRoute, useLensRoute } from "../../../route";
import { useTracesApi } from "../../api";
import type { TimeWindow } from "../../../../shared/timeRange/timeRange";

export function TraceTagFilters({
  accessToken = "",
  range,
  canInvestigate = true,
}: {
  accessToken?: string;
  range?: TimeWindow;
  canInvestigate?: boolean;
}) {
  const { tags, setTags } = useRunFilterRouting();
  const { setTab } = useLensRoute();
  const { openDialog } = useDialogRoute();
  const [key, setKey] = useState("");
  const [value, setValue] = useState("");
  const [requestTag, setRequestTag] = useState("");
  const [open, setOpen] = useState(false);
  const api = useTracesApi(accessToken);
  const tagValues = api.tagValues;
  const available = useQuery({
    queryKey: ["traceTagValues", accessToken, range],
    queryFn:
      range && tagValues ? ({ signal }) => tagValues(range, signal) : skipToken,
    enabled: open && !!range && !!api.tagValues,
    staleTime: 60_000,
    retry: false,
  });
  const addRequestTag = (value: string) => {
    if (!value.trim() || value.length > 256 || tags.length >= 8) return;
    if (!tags.some((tag) => tag.key === "tag" && tag.value === value))
      setTags([...tags, { key: "tag", value }]);
    setRequestTag("");
  };
  const add = () => {
    const condition = { key: key.trim(), value: value.trim() };
    if (!condition.key || !condition.value || tags.length >= 8) return;
    if (
      !tags.some(
        (tag) => tag.key === condition.key && tag.value === condition.value,
      )
    )
      setTags([...tags, condition]);
    setKey("");
    setValue("");
  };
  return (
    <div className="flex min-h-10 items-center gap-2 border-l px-2">
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger
          render={
            <Button variant="outline" size="sm" className="h-7 text-xs" />
          }
        >
          <Tags className="size-3.5" /> Tags
          {tags.length ? ` (${tags.length})` : ""}
        </PopoverTrigger>
        <PopoverContent
          align="start"
          className="max-h-[80vh] w-80 overflow-y-auto"
        >
          <PopoverTitle>Filter by tags</PopoverTitle>
          <p className="text-xs text-muted-foreground">
            Use the same tags recorded by LiteLLM. All selected tags must match.
          </p>
          {tags.map((tag, index) => (
            <div
              key={`${tag.key}:${tag.value}`}
              className="flex items-center justify-between gap-2 rounded border px-2 py-1"
            >
              <span className="break-all text-xs">
                {tag.key === "tag" ? tag.value : `${tag.key}: ${tag.value}`}
              </span>
              <Button
                variant="ghost"
                size="icon-xs"
                aria-label={`Remove tag filter ${tag.key}: ${tag.value}`}
                onClick={() => setTags(tags.filter((_, i) => i !== index))}
              >
                <X className="size-3" />
              </Button>
            </div>
          ))}
          <label className="grid gap-1 text-xs">
            LiteLLM tag
            <Input
              value={requestTag}
              maxLength={256}
              placeholder="workflow-diagrams"
              onChange={(event) => setRequestTag(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") addRequestTag(requestTag);
              }}
            />
          </label>
          <Button
            size="sm"
            disabled={!requestTag.trim() || tags.length >= 8}
            onClick={() => addRequestTag(requestTag)}
          >
            Add LiteLLM tag
          </Button>
          {available.isFetching && (
            <p className="text-xs text-muted-foreground">
              Loading LiteLLM tags…
            </p>
          )}
          {available.error && (
            <p className="text-xs text-muted-foreground">
              Could not load tags. Enter a tag to filter.
            </p>
          )}
          {!!available.data?.length && (
            <div
              className="grid max-h-40 gap-1 overflow-auto"
              aria-label="Available LiteLLM tags"
            >
              <p className="text-xs text-muted-foreground">
                From LiteLLM in this time range
              </p>
              {available.data.map((value) => (
                <Button
                  key={value}
                  variant="ghost"
                  size="sm"
                  className="h-auto justify-start whitespace-normal break-all text-left text-xs"
                  disabled={
                    tags.length >= 8 ||
                    tags.some((tag) => tag.key === "tag" && tag.value === value)
                  }
                  onClick={() => addRequestTag(value)}
                >
                  {value}
                </Button>
              ))}
            </div>
          )}
          <details className="text-xs">
            <summary className="cursor-pointer text-muted-foreground">
              Filter by metadata or Lens tags
            </summary>
            <div className="mt-3 grid gap-3">
              <label className="grid gap-1 text-xs">
                Tag key
                <Input
                  value={key}
                  maxLength={256}
                  onChange={(event) => setKey(event.target.value)}
                  placeholder="skill"
                />
              </label>
              <label className="grid gap-1 text-xs">
                Tag value
                <Input
                  value={value}
                  maxLength={256}
                  onChange={(event) => setValue(event.target.value)}
                  placeholder="diagram"
                  onKeyDown={(event) => {
                    if (event.key === "Enter") add();
                  }}
                />
              </label>
              <Button
                size="sm"
                disabled={!key.trim() || !value.trim() || tags.length >= 8}
                onClick={add}
              >
                Add tag filter
              </Button>
            </div>
          </details>
          {tags.length > 0 && (
            <Button variant="ghost" size="sm" onClick={() => setTags([])}>
              Clear tag filters
            </Button>
          )}
        </PopoverContent>
      </Popover>
      {canInvestigate && tags.length > 0 && (
        <Button
          variant="ghost"
          size="sm"
          className="h-7 text-xs"
          title="Investigate these tags and this agent over the selected lookback. Search, status and timeline zoom are not applied."
          onClick={() => {
            setTab("findings");
            openDialog("new");
          }}
        >
          Investigate tags
        </Button>
      )}
    </div>
  );
}
