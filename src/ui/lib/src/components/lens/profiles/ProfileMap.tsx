"use client";

import { useMemo } from "react";
import { Cell, LabelList, ResponsiveContainer, Scatter, ScatterChart, Tooltip, XAxis, YAxis, ZAxis } from "recharts";
import { formatCount, MUTED } from "./profilesData";
import type { ProfilesOverview } from "./schemas";

const DOMAIN: [number, number] = [0, 100];

const bubbleOf = (value: unknown): { name: string; countLabel: string } | null => {
  if (typeof value !== "object" || value === null) return null;
  if (!("name" in value) || !("countLabel" in value)) return null;
  return { name: String(value.name), countLabel: String(value.countLabel) };
};

function BubbleTooltip({ active, payload }: { active?: boolean; payload?: ReadonlyArray<{ payload?: unknown }> }) {
  const bubble = active ? bubbleOf(payload?.[0]?.payload) : null;
  if (!bubble) return null;
  return (
    <div className="rounded-md border bg-popover px-2 py-1 text-xs text-popover-foreground shadow-sm">
      <p className="font-medium">{bubble.name}</p>
      <p className="text-muted-foreground">{bubble.countLabel} runs</p>
    </div>
  );
}

export function ProfileMap({
  overview,
  colors,
  selected,
  onSelect,
}: {
  overview: ProfilesOverview;
  colors: ReadonlyMap<string, string>;
  selected: string | null;
  onSelect: (profileId: string) => void;
}) {
  const bubbles = useMemo(
    () =>
      overview.profiles.map((profile) => ({
        profile_id: profile.profile_id,
        name: profile.name,
        x: profile.center.x,
        y: profile.center.y,
        z: profile.run_count,
        countLabel: formatCount(profile.run_count),
      })),
    [overview.profiles],
  );
  const points = useMemo(() => overview.points.map((point) => ({ ...point, z: 1 })), [overview.points]);
  const discovered = overview.points.some((point) => point.placement === "discovery");
  return (
    <figure className="flex flex-col gap-2">
      <div
        role="img"
        aria-label={`Profile map with ${bubbles.length} profiles. Use the table for exact figures.`}
        className="h-80 w-full"
      >
        <ResponsiveContainer width="100%" height="100%">
          <ScatterChart margin={{ top: 24, right: 24, bottom: 12, left: 24 }} accessibilityLayer={false}>
            <Tooltip cursor={false} content={<BubbleTooltip />} />
            <XAxis type="number" dataKey="x" domain={DOMAIN} hide />
            <YAxis type="number" dataKey="y" domain={DOMAIN} hide reversed />
            <ZAxis zAxisId="points" type="number" dataKey="z" range={[14, 14]} />
            <ZAxis zAxisId="bubbles" type="number" dataKey="z" range={[220, 2600]} />
            <Scatter data={points} zAxisId="points" isAnimationActive={false} legendType="none">
              {points.map((point, index) => {
                const color = colors.get(point.profile_id) ?? MUTED;
                return (
                  <Cell
                    key={`${index}:${point.trace_id}`}
                    fill={color}
                    fillOpacity={point.placement === "discovery" ? 0.55 : 0}
                    stroke={color}
                    strokeOpacity={0.6}
                  />
                );
              })}
            </Scatter>
            <Scatter data={bubbles} zAxisId="bubbles" isAnimationActive={false} legendType="none">
              {bubbles.map((bubble) => (
                <Cell
                  key={bubble.profile_id}
                  fill={colors.get(bubble.profile_id) ?? MUTED}
                  fillOpacity={selected === null || selected === bubble.profile_id ? 0.32 : 0.12}
                  stroke={colors.get(bubble.profile_id) ?? MUTED}
                  strokeWidth={selected === bubble.profile_id ? 3 : 1.5}
                  className="cursor-pointer"
                  onClick={() => onSelect(bubble.profile_id)}
                />
              ))}
              <LabelList dataKey="countLabel" position="center" className="pointer-events-none fill-foreground text-xs font-medium" />
            </Scatter>
          </ScatterChart>
        </ResponsiveContainer>
      </div>
      <figcaption className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
        <span>
          {discovered
            ? "Bubble size is runs. Discovery sample positions come from embedding similarity. Other runs are placed around their profile."
            : "Bubble size is runs. Profiles are placed by rank, so position and distance carry no meaning."}
        </span>
        {discovered && (
          <span className="flex items-center gap-1.5">
            <span aria-hidden="true" className="size-2 rounded-full bg-muted-foreground/60" />
            Discovery sample position
          </span>
        )}
        <span className="flex items-center gap-1.5">
          <span aria-hidden="true" className="size-2 rounded-full border border-muted-foreground" />
          Placed around its profile
        </span>
        <span>{formatCount(points.length)} runs shown</span>
      </figcaption>
    </figure>
  );
}
