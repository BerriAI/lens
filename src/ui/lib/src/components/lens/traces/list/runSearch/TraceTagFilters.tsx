"use client";

import { useState } from "react";
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

export function TraceTagFilters({ canInvestigate = true }: { canInvestigate?: boolean }) {
  const { tags, setTags } = useRunFilterRouting();
  const { setTab } = useLensRoute();
  const { openDialog } = useDialogRoute();
  const [key, setKey] = useState("");
  const [value, setValue] = useState("");
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
      <Popover>
        <PopoverTrigger
          render={
            <Button variant="outline" size="sm" className="h-7 text-xs" />
          }
        >
          <Tags className="size-3.5" /> Tags
          {tags.length ? ` (${tags.length})` : ""}
        </PopoverTrigger>
        <PopoverContent align="start" className="w-80">
          <PopoverTitle>Filter by tags</PopoverTitle>
          <p className="text-xs text-muted-foreground">
            Match manual tags or recorded metadata on a run. All tags must match.
          </p>
          {tags.map((tag, index) => (
            <div
              key={`${tag.key}:${tag.value}`}
              className="flex items-center justify-between gap-2 rounded border px-2 py-1"
            >
              <span className="break-all text-xs">
                {tag.key}: {tag.value}
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
