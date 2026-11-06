"use client";

import { useMemo } from "react";
import { skipToken, useQuery } from "@tanstack/react-query";
import { Tags, X } from "lucide-react";

import type { TraceSummary } from "../../types";
import type { TimeWindow } from "../../../../shared/timeRange/timeRange";

import { SearchBox } from "../../../../shared/search/SearchBox";
import { itemValues } from "../../../../shared/search/valueSource";
import { RUN_INDEX, RUN_QUERY, type RunField } from "./runQuery";
import { runQueryCommand } from "./runSql";
import { useTracesApi } from "../../api";
import { useRunFilterRouting } from "../../routing";
import { useDialogRoute, useLensRoute } from "../../../route";
import { Button } from "../../../../ui/button";
import type { ValueSource } from "../../../../shared/search/valueSource";
import { searchTags } from "./tagSearch";

const COPY_HINT =
  "Copy this list as a curl call to the trace query API, which takes full SQL over the same rows.";

interface RunSearchProps {
  accessToken?: string;
  canInvestigate?: boolean;
  value: string;
  onChange: (value: string) => void;
  /** Loaded runs, the source of value suggestions. */
  runs: readonly TraceSummary[];
  /** The range the list shows; the copied query bounds itself to it. */
  range?: TimeWindow;
  busy?: boolean;
}

/** The runs list query box: free text plus `key:value` filters over run fields, copyable as a trace query. */
export function RunSearch({
  accessToken = "",
  canInvestigate = true,
  value,
  onChange,
  runs,
  range,
  busy = false,
}: RunSearchProps) {
  const command = useMemo(() => runQueryCommand(range), [range]);
  const api = useTracesApi(accessToken);
  const { tags, setTags } = useRunFilterRouting();
  const typedTags = searchTags(value);
  const chips = tags.filter(
    (tag) =>
      !typedTags.some(
        (typed) => typed.key === tag.key && typed.value === tag.value,
      ),
  );
  const { setTab } = useLensRoute();
  const { openDialog } = useDialogRoute();
  const localValues = itemValues(RUN_INDEX, runs);
  const tagValues = api.tagValues;
  const values: ValueSource<RunField> = {
    useValues(field, prefix) {
      const query = useQuery({
        queryKey: ["traceTagValues", accessToken, range],
        queryFn:
          range && tagValues
            ? ({ signal }) => tagValues(range, signal)
            : skipToken,
        enabled: field === "tag" && !!range && !!tagValues,
        staleTime: 60_000,
        retry: false,
      });
      return field === "tag"
        ? { values: query.data ?? [], loading: query.isFetching }
        : localValues.useValues(field, prefix);
    },
  };
  return (
    <div className="flex min-w-[min(100%,16rem)] basis-full flex-wrap items-center gap-1 sm:basis-80">
      <SearchBox.Root
        language={RUN_QUERY}
        values={values}
        value={value}
        onValueChange={onChange}
        label="Search runs"
        className="h-10 min-w-48 flex-1"
      >
        <SearchBox.Input
          className="h-full overflow-hidden rounded-none border-0 px-3 whitespace-nowrap focus-within:bg-muted/40 focus-within:ring-0 dark:bg-transparent"
          placeholder="Search runs or filter by tag"
          busy={busy}
        />
        <SearchBox.Suggestions>
          <SearchBox.CopyCommand
            title={COPY_HINT}
            command={command}
            supports={(query) =>
              tags.length === 0 &&
              !query.filters.some((filter) => filter.field === "tag")
            }
          />
        </SearchBox.Suggestions>
      </SearchBox.Root>
      {chips.map((tag) => (
        <Button
          key={`${tag.key}:${tag.value}`}
          variant="secondary"
          size="xs"
          className="mx-1 max-w-full"
          aria-label={`Remove tag filter ${tag.key}: ${tag.value}`}
          onClick={() =>
            setTags(
              tags.filter(
                (other) => other.key !== tag.key || other.value !== tag.value,
              ),
            )
          }
        >
          <Tags aria-hidden="true" className="size-3" />
          <span className="truncate">
            {tag.key === "tag" ? tag.value : `${tag.key}: ${tag.value}`}
          </span>
          <X aria-hidden="true" className="size-3" />
        </Button>
      ))}
      {canInvestigate && tags.length > 0 && (
        <Button
          variant="ghost"
          size="sm"
          className="mx-1 text-xs"
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
