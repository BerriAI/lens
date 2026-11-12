"use client";

import { CartesianGrid, ResponsiveContainer, Scatter, ScatterChart, Tooltip, XAxis, YAxis } from "recharts";
import { comparisonRows, paretoFrontier, recommendedModel, type Optimization } from "./model";

type ModelPoint = ReturnType<typeof comparisonRows>[number];
const tick = { fontSize: 11, fill: "var(--muted-foreground)" };
const dollars = (value: number) => `$${value.toFixed(3)}`;

export function CostQualityPlot({ run, unit }: { run: Optimization; unit: string }) {
  const points = comparisonRows(run)
    .filter((row) => row.scored > 0)
    .toSorted((a, b) => a.cost - b.cost);
  if (!points.length) return null;
  const recommended = recommendedModel(run)?.id;
  const frontier = paretoFrontier(points);
  const maxCost = Math.ceil((Math.max(...points.map((row) => row.cost)) * 1.15) / 0.005) * 0.005;
  const qualityDomain = [
    Math.max(0, Math.floor((Math.min(...points.map((row) => row.quality)) - 0.25) * 2) / 2),
    Math.min(10, Math.ceil((Math.max(...points.map((row) => row.quality)) + 0.25) * 2) / 2),
  ];
  const qualityLabel = run.config.judge === "feedback" ? "Feedback / 10" : "Judge score / 10";
  const costTicks = Array.from({ length: Math.round(maxCost / 0.005) + 1 }, (_, index) => index * 0.005);
  const qualityTicks = Array.from(
    { length: Math.round((qualityDomain[1] - qualityDomain[0]) / 0.5) + 1 },
    (_, index) => qualityDomain[0] + index * 0.5,
  );
  return (
    <section className="grid gap-4" aria-label="Cost vs. quality">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h3 className="text-sm font-medium">Cost vs. quality</h3>
        <div className="flex flex-wrap items-center gap-4 text-xs text-muted-foreground">
          <span className="flex items-center gap-1.5">
            <span className="size-2 rounded-full border-2 border-foreground" />
            Current model
          </span>
          {recommended && (
            <span className="flex items-center gap-1.5">
              <span className="size-2 rounded-full bg-primary" />
              Recommended
            </span>
          )}
          <span className="flex items-center gap-1.5">
            <span className="w-5 border-t border-dashed border-muted-foreground" />
            Best tradeoffs
          </span>
        </div>
      </div>
      <div className="overflow-x-auto">
        <div className="min-w-[480px]">
          <p className="text-xs text-muted-foreground">{qualityLabel}</p>
          <div className="h-64 w-full">
            <ResponsiveContainer width="100%" height="100%">
              <ScatterChart margin={{ left: 0, right: 20, top: 26, bottom: 8 }}>
                <CartesianGrid vertical={false} stroke="var(--border)" strokeOpacity={0.6} />
                <XAxis
                  type="number"
                  dataKey="cost"
                  name={`Cost / ${unit}`}
                  domain={[0, maxCost]}
                  tickFormatter={dollars}
                  tick={tick}
                  axisLine={false}
                  tickLine={false}
                  tickMargin={10}
                  ticks={costTicks}
                />
                <YAxis
                  type="number"
                  dataKey="quality"
                  name={qualityLabel}
                  domain={qualityDomain}
                  tick={tick}
                  axisLine={false}
                  tickLine={false}
                  width={36}
                  ticks={qualityTicks}
                />
                <Tooltip
                  cursor={false}
                  content={({ active, payload }) => {
                    const raw: unknown = payload?.[0]?.payload;
                    const point =
                      raw && typeof raw === "object" && "id" in raw
                        ? points.find((item) => item.id === raw.id)
                        : undefined;
                    return active && point ? (
                      <div className="grid gap-1.5 rounded-lg border bg-popover px-3 py-2 text-xs text-popover-foreground shadow-sm">
                        <p className="font-mono">{point.id}</p>
                        <p>
                          {qualityLabel}: {point.quality.toFixed(2)}
                        </p>
                        <p>
                          Cost / {unit}: ${point.cost.toFixed(4)}
                        </p>
                        <p className="text-muted-foreground">{point.scored.toLocaleString("en-US")} scored</p>
                      </div>
                    ) : null;
                  }}
                />
                <Scatter
                  data={frontier}
                  line={{ stroke: "var(--muted-foreground)", strokeWidth: 1, strokeDasharray: "3 5" }}
                  shape={() => <g />}
                  tooltipType="none"
                  isAnimationActive={false}
                />
                {points.map((point, index) => (
                  <Scatter
                    key={point.id}
                    name={point.id}
                    data={[point]}
                    shape={
                      <Point
                        point={point}
                        recommended={point.id === recommended}
                        below={index % 2 === 1}
                        alignRight={point.cost / maxCost > 0.75}
                      />
                    }
                    isAnimationActive={false}
                  />
                ))}
              </ScatterChart>
            </ResponsiveContainer>
          </div>
          <p className="text-right text-xs text-muted-foreground">Cost / {unit}</p>
        </div>
      </div>
    </section>
  );
}

function Point({
  cx,
  cy,
  point,
  recommended,
  below,
  alignRight,
}: {
  cx?: number;
  cy?: number;
  point: ModelPoint;
  recommended: boolean;
  below: boolean;
  alignRight: boolean;
}) {
  if (cx === undefined || cy === undefined) return null;
  const color = recommended ? "var(--primary)" : point.isBaseline ? "var(--foreground)" : "var(--muted-foreground)";
  return (
    <g>
      {recommended && <circle cx={cx} cy={cy} r={10} fill={color} fillOpacity={0.1} />}
      <circle
        cx={cx}
        cy={cy}
        r={4.5}
        fill={point.isBaseline ? "var(--background)" : color}
        stroke={color}
        strokeWidth={2}
      />
      <text
        x={cx + (alignRight ? -9 : 9)}
        y={cy + (below ? 22 : -13)}
        textAnchor={alignRight ? "end" : "start"}
        fill={recommended ? "var(--primary)" : "var(--foreground)"}
        className="font-mono text-[11px]"
      >
        {point.id}
      </text>
    </g>
  );
}
