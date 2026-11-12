"use client";

import { useMemo, useState } from "react";
import { cn } from "../../../lib/cva.config";
import { NUMERIC_CELL_CLASS, Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "../../ui/table";
import { formatCount, formatShare, formatUsd, MUTED, SOURCE_LABELS } from "./profilesData";
import type { ProfileSummary } from "./schemas";

const RANKS = ["runs", "spend"] as const;
type Rank = (typeof RANKS)[number];

const ranked = (profiles: readonly ProfileSummary[], rank: Rank) =>
  [...profiles].sort((a, b) =>
    rank === "runs" ? b.run_count - a.run_count : (b.spend_usd ?? -1) - (a.spend_usd ?? -1),
  );

export function ProfileTable({
  profiles,
  colors,
  selected,
  onSelect,
}: {
  profiles: readonly ProfileSummary[];
  colors: ReadonlyMap<string, string>;
  selected: string | null;
  onSelect: (profileId: string) => void;
}) {
  const [rank, setRank] = useState<Rank>("runs");
  const rows = useMemo(() => ranked(profiles, rank), [profiles, rank]);
  return (
    <div className="flex flex-col gap-2">
      <div role="radiogroup" aria-label="Rank profiles by" className="flex w-fit rounded-lg bg-muted p-1">
        {RANKS.map((option) => (
          <button
            key={option}
            type="button"
            role="radio"
            aria-checked={rank === option}
            className={cn(
              "rounded-md px-3 py-1 text-xs transition-colors",
              rank === option ? "bg-card text-foreground shadow-xs" : "text-muted-foreground hover:text-foreground",
            )}
            onClick={() => setRank(option)}
          >
            {option === "runs" ? "Runs" : "Spend"}
          </button>
        ))}
      </div>
      <Table aria-label="Profiles ranked by runs or spend">
        <TableHeader>
          <TableRow>
            <TableHead>Profile</TableHead>
            <TableHead className={NUMERIC_CELL_CLASS}>Runs</TableHead>
            <TableHead className={NUMERIC_CELL_CLASS}>Spend</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.map((profile) => (
            <TableRow key={profile.profile_id} data-state={selected === profile.profile_id ? "selected" : undefined}>
              <TableCell className="whitespace-normal">
                <button
                  type="button"
                  className="flex items-start gap-2 text-left hover:underline"
                  onClick={() => onSelect(profile.profile_id)}
                >
                  <span
                    aria-hidden="true"
                    className="mt-1.5 size-2.5 shrink-0 rounded-full"
                    style={{ backgroundColor: colors.get(profile.profile_id) ?? MUTED }}
                  />
                  <span className="min-w-0">
                    <span className="block font-medium break-words">{profile.name}</span>
                    <span className="block text-xs text-muted-foreground">{SOURCE_LABELS[profile.source]}</span>
                  </span>
                </button>
              </TableCell>
              <TableCell className={NUMERIC_CELL_CLASS}>
                <span>{formatCount(profile.run_count)}</span>
                <span className="block text-xs text-muted-foreground">{formatShare(profile.share)}</span>
              </TableCell>
              <TableCell className={`${NUMERIC_CELL_CLASS} whitespace-normal`}>
                <span>{formatUsd(profile.spend_usd) ?? "No data"}</span>
                {profile.cost_observations < profile.request_count && (
                  <span className="block text-xs text-muted-foreground">
                    Cost known for {formatCount(profile.cost_observations)} of {formatCount(profile.request_count)} requests
                  </span>
                )}
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  );
}
