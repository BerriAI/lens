"use client";

import { useMemo, useState } from "react";
import { cn } from "../../../lib/cva.config";
import { SegmentedControl } from "./SegmentedControl";
import {
  formatCount,
  formatShare,
  formatUsd,
  NEUTRAL,
  rankProfiles,
  type Rank,
  type RankedProfile,
  SOURCE_LABELS,
} from "./profilesData";
import type { ProfileSummary } from "./schemas";

const RANK_OPTIONS = [
  { value: "runs", label: "Runs" },
  { value: "spend", label: "Spend" },
] as const;

function figures({ profile }: RankedProfile, rank: Rank): { primary: string; secondary: string } {
  const spend = formatUsd(profile.spend_usd);
  const partial = profile.cost_observations < profile.request_count;
  const costNote = `cost known for ${formatCount(profile.cost_observations)} of ${formatCount(profile.request_count)} requests`;
  if (rank === "runs") {
    return {
      primary: `${formatCount(profile.run_count)} ${profile.run_count === 1 ? "run" : "runs"}`,
      secondary: `${formatShare(profile.share)} · ${spend ?? "no cost data"}`,
    };
  }
  return {
    primary: spend ?? "No cost data",
    secondary: partial ? costNote : `${formatCount(profile.run_count)} runs`,
  };
}

export function ProfileBreakdown({
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
  const rows = useMemo(() => rankProfiles(profiles, rank), [profiles, rank]);
  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <SegmentedControl label="Rank profiles by" options={RANK_OPTIONS} value={rank} onChange={setRank} />
        <span className="text-xs text-muted-foreground">
          {formatCount(profiles.length)} {profiles.length === 1 ? "profile" : "profiles"}
        </span>
      </div>
      <ol aria-label={`Profiles ranked by ${rank}`} className="-mx-2 max-h-[26rem] space-y-0.5 overflow-y-auto">
        {rows.map((row) => {
          const { profile } = row;
          const active = selected === profile.profile_id;
          const { primary, secondary } = figures(row, rank);
          return (
            <li key={profile.profile_id}>
              <button
                type="button"
                aria-current={active || undefined}
                onClick={() => onSelect(profile.profile_id)}
                className={cn(
                  "w-full rounded-md px-2 py-2 text-left transition-[background-color,opacity] duration-150 hover:bg-muted/60 focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none motion-reduce:transition-none",
                  active && "bg-muted",
                  selected !== null && !active && "opacity-55 hover:opacity-100",
                )}
              >
                <span className="flex flex-col gap-1 sm:flex-row sm:items-start sm:justify-between sm:gap-4">
                  <span className="min-w-0 sm:flex-1">
                    <span className="line-clamp-2 text-sm font-medium break-words" title={profile.name}>
                      {profile.name}
                    </span>
                    <span className="block text-xs text-muted-foreground">{SOURCE_LABELS[profile.source]}</span>
                  </span>
                  <span className="min-w-0 tabular-nums sm:max-w-[45%] sm:text-right">
                    <span className="block text-sm">{primary}</span>
                    <span className="block text-xs break-words text-muted-foreground">{secondary}</span>
                  </span>
                </span>
                {row.value === null ? (
                  <span aria-hidden="true" className="mt-2 block h-1.5 rounded-full border border-dashed border-muted-foreground/40" />
                ) : (
                  <span aria-hidden="true" className="mt-2 block h-1.5 overflow-hidden rounded-full bg-muted">
                    <span
                      className="block h-full rounded-full"
                      style={{
                        width: `${(row.fraction * 100).toFixed(1)}%`,
                        backgroundColor: colors.get(profile.profile_id) ?? NEUTRAL,
                      }}
                    />
                  </span>
                )}
              </button>
            </li>
          );
        })}
      </ol>
    </div>
  );
}
