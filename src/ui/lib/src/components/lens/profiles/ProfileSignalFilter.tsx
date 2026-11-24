"use client";

import { Check, ListFilter, Search, X } from "lucide-react";
import { useState } from "react";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Popover, PopoverContent, PopoverTitle, PopoverTrigger } from "../../ui/popover";

export function ProfileSignalFilter({
  signals,
  value,
  loading,
  error,
  onChange,
  onRetry,
}: {
  signals: readonly { id: string; name: string }[];
  value: string | null;
  loading: boolean;
  error: boolean;
  onChange: (value: string | null) => void;
  onRetry: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const selected = signals.find((signal) => signal.id === value);
  const shown = signals.filter((signal) => signal.name.toLowerCase().includes(search.trim().toLowerCase()));
  const change = (next: string | null) => {
    onChange(next);
    setOpen(false);
    setSearch("");
  };
  return (
    <div aria-label="Profile filters" className="flex flex-wrap items-center gap-2 border-b pb-4">
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger render={<Button variant="outline" size="sm" />}>
          <ListFilter aria-hidden="true" className="size-3.5" /> Filter
        </PopoverTrigger>
        {value && (
          <div className="inline-flex max-w-full items-stretch rounded-md border bg-muted/40">
            <PopoverTrigger
              aria-label={`Edit signal filter: ${selected?.name ?? value}`}
              className="flex min-w-0 flex-wrap items-center gap-1.5 rounded-l-md px-2.5 py-1.5 text-xs hover:bg-muted focus-visible:outline-2 focus-visible:outline-ring"
            >
              <span className="text-muted-foreground">Signal</span>
              <span className="break-words font-medium">{selected?.name ?? value}</span>
              <span className="text-muted-foreground">is</span>
              <span>Flagged</span>
            </PopoverTrigger>
            <Button variant="ghost" size="icon-sm" aria-label="Remove signal filter" className="h-auto shrink-0 rounded-l-none border-l" onClick={() => change(null)}>
              <X aria-hidden="true" className="size-3.5" />
            </Button>
          </div>
        )}
        <PopoverContent align="start" className="max-w-[calc(100vw-2rem)] gap-1.5 p-2">
          <PopoverTitle className="px-2 py-1 text-xs">Runs flagged by signal</PopoverTitle>
          <div className="relative">
            <Search aria-hidden="true" className="absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
            <Input aria-label="Find a signal" placeholder="Find a signal…" autoFocus value={search} onChange={(event) => setSearch(event.target.value)} className="h-8 pl-8 text-sm" />
          </div>
          {loading && <p role="status" className="px-2 py-2 text-xs text-muted-foreground">Loading signals…</p>}
          {error ? (
            <div role="alert" className="px-2 py-2 text-xs">
              Could not load signals
              <Button variant="ghost" size="sm" onClick={onRetry}>Retry</Button>
            </div>
          ) : (
            <ul aria-label="Signals" className="max-h-64 overflow-y-auto">
              {shown.map((signal) => (
                <li key={signal.id}>
                  <Button variant="ghost" className="h-auto w-full justify-start gap-2 px-2 py-2 text-left font-normal whitespace-normal" onClick={() => change(signal.id)}>
                    <Check aria-hidden="true" className={`size-3.5 shrink-0 ${value !== signal.id ? "invisible" : ""}`} />
                    <span className="min-w-0 break-words">{signal.name}</span>
                  </Button>
                </li>
              ))}
              {!loading && shown.length === 0 && <li className="px-2 py-2 text-xs text-muted-foreground">{signals.length ? "No signals match" : "No signals configured"}</li>}
            </ul>
          )}
          <p className="border-t px-2 pt-2 pb-1 text-xs text-muted-foreground">Uses saved results and the configured threshold</p>
        </PopoverContent>
      </Popover>
    </div>
  );
}
