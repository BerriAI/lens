"use client";

import {
  Building2,
  ChevronRight,
  Folder,
  KeyRound,
  Loader2,
  Users,
} from "lucide-react";
import { useState } from "react";
import {
  Combobox,
  ComboboxContent,
  ComboboxEmpty,
  ComboboxInput,
  ComboboxItem,
  ComboboxList,
  ComboboxTrigger,
} from "../../ui/combobox";
import { LensBrand } from "../LensBrand";
import { useLensHost } from "../../../host/LensHost";
import type { WorkspaceItem, WorkspaceKind } from "./scope";

export type { WorkspaceItem, WorkspaceKind } from "./scope";

export interface WorkspacePicker {
  readonly kind: WorkspaceKind;
  readonly value: WorkspaceItem | null;
  readonly items: readonly WorkspaceItem[];
  readonly loading?: boolean;
  readonly error?: string | null;
  readonly query?: string;
  readonly onSearch?: (query: string) => void;
  readonly hasMore?: boolean;
  readonly onLoadMore?: () => void;
  readonly onRetry?: () => void;
  readonly emptyLabel?: string;
}

const WORKSPACES = {
  organization: {
    label: "Organization",
    all: "All organizations",
    Icon: Building2,
  },
  team: { label: "Team", all: "All teams", Icon: Users },
  project: { label: "Project", all: "All projects", Icon: Folder },
  key: { label: "Key", all: "All keys", Icon: KeyRound },
} as const;

function WorkspaceSelect({
  picker,
  noProject,
  onSelect,
}: {
  picker: WorkspacePicker;
  noProject: boolean;
  onSelect: (kind: WorkspaceKind, item: WorkspaceItem | null) => void;
}) {
  const [query, setQuery] = useState("");
  const { label, all, Icon } = WORKSPACES[picker.kind];
  const allItem: WorkspaceItem = { id: "", name: all };
  const items = [allItem, ...picker.items];
  const changeSearch = (next: string) => {
    setQuery(next);
    picker.onSearch?.(next);
  };
  return (
    <Combobox
      items={items}
      value={picker.value ?? allItem}
      inputValue={picker.query ?? query}
      itemToStringLabel={(item) => item.name}
      isItemEqualToValue={(item, value) => item.id === value.id}
      filter={picker.onSearch ? null : undefined}
      onInputValueChange={(next, details) => {
        if (
          details.reason === "input-change" ||
          details.reason === "input-clear"
        )
          changeSearch(next);
      }}
      onOpenChange={(open) => {
        if (!open && (picker.query ?? query)) changeSearch("");
      }}
      onValueChange={(item) => onSelect(picker.kind, item?.id ? item : null)}
    >
      <ComboboxTrigger
        aria-label={`${label} workspace`}
        className="group flex min-w-0 flex-1 items-center gap-2 rounded-md px-2.5 py-2 text-left outline-none transition-colors hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring data-popup-open:bg-accent [&>svg:last-child]:ml-auto [&>svg:last-child]:size-3.5"
      >
        <Icon
          aria-hidden="true"
          className="size-4 shrink-0 text-muted-foreground"
        />
        <span className="min-w-0 flex-1">
          <span className="block text-[11px] leading-4 text-muted-foreground">
            {label}
          </span>
          <span className="block truncate text-[13px] leading-5 font-medium">
            {picker.value?.name ??
              (noProject ? "No project" : (picker.emptyLabel ?? all))}
          </span>
        </span>
        {picker.loading && (
          <Loader2
            aria-label={`Loading ${label.toLowerCase()}s`}
            className="size-3 animate-spin"
          />
        )}
      </ComboboxTrigger>
      <ComboboxContent className="min-w-64 max-w-80" align="start">
        <ComboboxInput
          aria-label={`Search ${label.toLowerCase()}s`}
          placeholder={`Search ${label.toLowerCase()}s…`}
          showTrigger={false}
        />
        {picker.error ? (
          <div role="alert" className="px-3 py-3 text-xs text-muted-foreground">
            {picker.error}
            {picker.onRetry && (
              <button
                type="button"
                onClick={picker.onRetry}
                className="ml-2 text-foreground underline"
              >
                Retry
              </button>
            )}
          </div>
        ) : (
          <>
            <ComboboxEmpty>
              {picker.loading ? "Loading…" : "No matches"}
            </ComboboxEmpty>
            <ComboboxList>
              {(item: WorkspaceItem) => (
                <ComboboxItem key={item.id} value={item} className="py-2">
                  <Icon aria-hidden="true" className="text-muted-foreground" />
                  <span className="min-w-0">
                    <span className="block truncate">{item.name}</span>
                    {item.description && (
                      <span className="block truncate text-xs text-muted-foreground">
                        {item.description}
                      </span>
                    )}
                  </span>
                </ComboboxItem>
              )}
            </ComboboxList>
          </>
        )}
        {picker.hasMore && picker.onLoadMore && (
          <button
            type="button"
            className="w-full border-t px-3 py-2 text-xs text-muted-foreground hover:bg-accent disabled:opacity-50"
            onClick={picker.onLoadMore}
            disabled={picker.loading}
          >
            {picker.loading ? "Loading…" : "Load more"}
          </button>
        )}
      </ComboboxContent>
    </Combobox>
  );
}

export function WorkspaceSwitchers({
  pickers,
  onSelect,
}: {
  readonly pickers: readonly WorkspacePicker[];
  readonly onSelect: (kind: WorkspaceKind, item: WorkspaceItem | null) => void;
}) {
  const embedded = useLensHost().surface === "embedded";
  const key = pickers.find((picker) => picker.kind === "key")?.value;
  return (
    <header
      aria-label="Lens workspace"
      className="flex shrink-0 flex-col gap-2 border-b bg-background px-3 py-2 sm:flex-row sm:items-center sm:gap-4 sm:px-4"
    >
      {!embedded && (
        <h1 className="px-1">
          <LensBrand showLiteLLM={false} className="[&_.lens-brand-name]:text-lg" />
        </h1>
      )}
      <nav
        aria-label="Workspace hierarchy"
        className="grid min-w-0 flex-1 grid-cols-2 gap-1 sm:flex sm:items-center"
      >
        {pickers.map((picker, index) => (
          <div key={picker.kind} className="flex min-w-0 flex-1 items-center">
            {index > 0 && (
              <ChevronRight
                aria-hidden="true"
                className="mr-1 hidden size-3 shrink-0 text-muted-foreground/50 sm:block"
              />
            )}
            <WorkspaceSelect
              picker={picker}
              noProject={picker.kind === "project" && key?.project_id === null}
              onSelect={onSelect}
            />
          </div>
        ))}
      </nav>
    </header>
  );
}
