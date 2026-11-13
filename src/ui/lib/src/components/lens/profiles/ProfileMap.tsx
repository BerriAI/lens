"use client";

import { useMemo, useRef, useState, type KeyboardEvent, type RefObject } from "react";
import { useResizeObserver } from "usehooks-ts";
import { cn } from "../../../lib/cva.config";
import { estimateWidth, LABEL_FONT, layoutMap, type Measure } from "./profileMapLayout";
import { Button } from "../../ui/button";
import { formatCount, NEUTRAL, similarity, SOURCE_LABELS } from "./profilesData";
import type { ProfilesOverview } from "./schemas";

const MEASURE_MARGIN = 1.08;

function canvasMeasure(element: () => HTMLElement | null): Measure {
  const context = typeof document === "undefined" ? null : document.createElement("canvas").getContext("2d");
  if (!context) return estimateWidth;
  return (text, font) => {
    const target = element();
    if (!target) return estimateWidth(text, font);
    const family = getComputedStyle(target).fontFamily || "sans-serif";
    context.font = font === "name" ? `500 ${LABEL_FONT.name}px ${family}` : `400 ${LABEL_FONT.count}px ${family}`;
    return context.measureText(text).width * MEASURE_MARGIN;
  };
}

const activate = (event: KeyboardEvent, action: () => void) => {
  if (event.key !== "Enter" && event.key !== " ") return;
  event.preventDefault();
  action();
};

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
  const data = useMemo(() => similarity(overview), [overview]);
  const areaRef = useRef<HTMLDivElement>(null);
  const { width = 0 } = useResizeObserver({ ref: areaRef as RefObject<HTMLDivElement>, box: "content-box" });
  const [hovered, setHovered] = useState<string | null>(null);
  const [zoomed, setZoomed] = useState(false);
  const height = Math.round(Math.min(380, Math.max(260, width * 0.6)));
  const measure = useMemo(() => canvasMeasure(() => areaRef.current), []);
  const selectedOnMap = data.groups.some((group) => group.profile.profile_id === selected);
  const zoom = zoomed && selectedOnMap ? selected : null;
  const layout = useMemo(
    () =>
      layoutMap(
        data.groups.map((group) => ({
          profileId: group.profile.profile_id,
          name: group.profile.name,
          count: group.points.length,
          points: group.points,
        })),
        { width, height },
        { measure, priority: selected, zoom },
      ),
    [data.groups, width, height, measure, selected, zoom],
  );
  const names = useMemo(() => new Map(data.groups.map((group) => [group.profile.profile_id, group.profile.name])), [data.groups]);
  const focus = hovered ?? selected;
  if (data.groups.length === 0) {
    return (
      <div role="status" className="rounded-lg border border-dashed px-4 py-8 text-center text-sm text-muted-foreground">
        <p className="font-medium text-foreground">No similarity positions in this range</p>
        <p className="mx-auto mt-1 max-w-md">
          Positions come from enhanced discovery. Runs in a frozen taxonomy&apos;s discovery sample are placed by how
          similar their requests are. Breakdown covers every run.
        </p>
      </div>
    );
  }
  return (
    <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_16rem]">
      <figure className="flex min-w-0 flex-col gap-2">
        <div ref={areaRef} className="w-full overflow-hidden rounded-lg border bg-muted/20" style={{ height }}>
          {width > 0 && (
            <svg
              width={width}
              height={height}
              role="group"
              aria-label={`Similarity map of ${formatCount(data.placed)} ${data.placed === 1 ? "run" : "runs"} in ${formatCount(data.groups.length)} ${data.groups.length === 1 ? "group" : "groups"}`}
              className="block font-sans select-none"
            >
              <g aria-hidden="true">
                {layout.groups.map((group) =>
                  group.points.map((point) => (
                    <circle
                      key={point.key}
                      cx={point.x}
                      cy={point.y}
                      r={focus === group.profileId ? 4.5 : 3.5}
                      className="cursor-pointer transition-opacity duration-150 motion-reduce:transition-none"
                      style={{ fill: colors.get(group.profileId) ?? NEUTRAL }}
                      opacity={focus === null ? 0.75 : focus === group.profileId ? 1 : 0.18}
                      onClick={() => onSelect(group.profileId)}
                      onMouseEnter={() => setHovered(group.profileId)}
                      onMouseLeave={() => setHovered(null)}
                    />
                  )),
                )}
                {layout.labels
                  .filter((label) => label.leader)
                  .map((label) => (
                    <line
                      key={`leader:${label.profileId}`}
                      x1={label.anchor.x}
                      y1={label.anchor.y}
                      x2={label.leader?.x}
                      y2={label.leader?.y}
                      className="stroke-border"
                      strokeWidth={1}
                    />
                  ))}
              </g>
              {layout.labels.map((label) => {
                const name = names.get(label.profileId) ?? "";
                const active = selected === label.profileId;
                const dimmed = focus !== null && focus !== label.profileId;
                return (
                  <g
                    key={label.profileId}
                    role="button"
                    tabIndex={0}
                    aria-label={`${name}, ${label.countText} on the map`}
                    aria-pressed={active}
                    className="cursor-pointer outline-none [&:focus-visible>rect]:stroke-ring"
                    opacity={dimmed ? 0.45 : 1}
                    onClick={() => onSelect(label.profileId)}
                    onKeyDown={(event) => activate(event, () => onSelect(label.profileId))}
                    onMouseEnter={() => setHovered(label.profileId)}
                    onMouseLeave={() => setHovered(null)}
                  >
                    <title>{name}</title>
                    <rect
                      x={label.box.x}
                      y={label.box.y}
                      width={label.box.width}
                      height={label.box.height}
                      rx={4}
                      className="fill-card"
                      style={{ stroke: active ? (colors.get(label.profileId) ?? NEUTRAL) : "var(--border)" }}
                      strokeWidth={active ? 1.5 : 1}
                    />
                    <text x={label.box.x + LABEL_FONT.padding} className="fill-foreground">
                      {label.lines.map((line, index) => (
                        <tspan
                          key={index}
                          x={label.box.x + LABEL_FONT.padding}
                          y={label.box.y + LABEL_FONT.padding + LABEL_FONT.name + index * LABEL_FONT.nameLine - 2}
                          fontSize={LABEL_FONT.name}
                          fontWeight={500}
                        >
                          {line}
                        </tspan>
                      ))}
                      <tspan
                        x={label.box.x + LABEL_FONT.padding}
                        y={label.box.y + LABEL_FONT.padding + label.lines.length * LABEL_FONT.nameLine + LABEL_FONT.count - 1}
                        fontSize={LABEL_FONT.count}
                        className="fill-muted-foreground"
                      >
                        {label.countText}
                      </tspan>
                    </text>
                  </g>
                );
              })}
            </svg>
          )}
        </div>
        <figcaption className="flex flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground">
          <span>
            {formatCount(data.placed)} of {formatCount(data.total)} runs on the map. Nearby points suggest similar
            requests; distances are approximate.
          </span>
          {selectedOnMap && (
            <Button variant="outline" size="xs" aria-pressed={zoomed} onClick={() => setZoomed(!zoomed)}>
              {zoomed ? "Show all groups" : "Zoom to selection"}
            </Button>
          )}
        </figcaption>
      </figure>
      <div className="flex min-w-0 flex-col gap-4 text-sm">
        <section aria-label="Groups on the map" className="flex flex-col gap-1">
          <h4 className="text-xs font-medium text-muted-foreground">On the map</h4>
          <ul className="flex flex-col gap-0.5">
            {data.groups.map((group) => {
              const id = group.profile.profile_id;
              return (
                <li key={id}>
                  <button
                    type="button"
                    aria-current={selected === id || undefined}
                    onClick={() => onSelect(id)}
                    onMouseEnter={() => setHovered(id)}
                    onMouseLeave={() => setHovered(null)}
                    className={cn(
                      "flex w-full items-start justify-between gap-3 rounded-md px-2 py-1 text-left transition-[background-color,opacity] duration-150 hover:bg-muted/60 motion-reduce:transition-none",
                      selected === id && "bg-muted",
                      focus !== null && focus !== id && "opacity-55",
                    )}
                  >
                    <span className="flex min-w-0 items-start gap-2">
                      <span
                        aria-hidden="true"
                        className="mt-1.5 size-2 shrink-0 rounded-full"
                        style={{ backgroundColor: colors.get(id) ?? NEUTRAL }}
                      />
                      <span className="min-w-0 break-words">{group.profile.name}</span>
                    </span>
                    <span className="shrink-0 text-muted-foreground tabular-nums">{formatCount(group.points.length)}</span>
                  </button>
                </li>
              );
            })}
          </ul>
        </section>
        {data.unplaced.length > 0 && (
          <section aria-label="Runs not on the map" className="flex flex-col gap-1">
            <h4 className="text-xs font-medium text-muted-foreground">Not on the map</h4>
            <ul className="flex flex-col gap-0.5">
              {data.unplaced.map(({ profile, runs }) => (
                <li key={profile.profile_id}>
                  <button
                    type="button"
                    onClick={() => onSelect(profile.profile_id)}
                    className="flex w-full items-start justify-between gap-3 rounded-md px-2 py-1 text-left hover:bg-muted/60"
                  >
                    <span className="min-w-0">
                      <span className="block break-words">{profile.name}</span>
                      <span className="block text-xs text-muted-foreground">{SOURCE_LABELS[profile.source]}</span>
                    </span>
                    <span className="shrink-0 text-muted-foreground tabular-nums">{formatCount(runs)}</span>
                  </button>
                </li>
              ))}
            </ul>
          </section>
        )}
      </div>
    </div>
  );
}
