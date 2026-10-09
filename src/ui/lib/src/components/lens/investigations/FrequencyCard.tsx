"use client";

import { Bar, BarChart, CartesianGrid, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import { dayLabel, type Frequency, percentLabel } from "../model/frequency";

const AFFECTED = "var(--finding-affected)";
const UNAFFECTED = "var(--finding-unaffected)";
const TICK = { fontSize: 11, fill: "var(--muted-foreground)" };
const TOP_RADIUS: [number, number, number, number] = [3, 3, 0, 0];

function Legend() {
  return (
    <div className="flex flex-wrap gap-x-4 gap-y-1 font-mono text-[10px] text-muted-foreground">
      <span className="flex items-center gap-1.5">
        <span aria-hidden="true" className="size-2 rounded-sm bg-finding-affected" />
        Affected
      </span>
      <span className="flex items-center gap-1.5">
        <span aria-hidden="true" className="size-2 rounded-sm bg-finding-unaffected" />
        Unaffected
      </span>
    </div>
  );
}

export function FrequencyCard({ frequency }: { frequency: Frequency }) {
  const percent = percentLabel(frequency.affected, frequency.total);
  const data = frequency.days.map((d) => ({ ...d, label: dayLabel(d.day) }));
  const range = data.length > 0 ? `${data[0].label} – ${data[data.length - 1].label}` : null;
  return (
    <section aria-label="Frequency" className="space-y-4 rounded-lg border border-t-2 border-t-indigo-400 bg-card p-4">
      <div aria-live="polite" className="flex flex-wrap items-start justify-between gap-4">
        <div>
          <h3 className="lens-section-label mb-2 font-mono text-[10px] font-medium text-muted-foreground">Frequency</h3>
          <p className="text-3xl leading-tight font-medium tracking-tight text-primary tabular-nums">
            {percent ?? "—"}
            <span className="mt-2 block font-mono text-[11px] font-normal tracking-normal text-muted-foreground">
              {frequency.affected} of {frequency.total} traces affected
            </span>
          </p>
        </div>
        {range && (
          <span className="rounded border bg-muted/20 px-2 py-1 font-mono text-[10px] whitespace-nowrap text-muted-foreground">
            {range}
          </span>
        )}
      </div>
      {data.length > 0 && (
        <div className="h-56 md:h-64" data-testid="frequency-chart">
          <ResponsiveContainer width="100%" height="100%">
            <BarChart
              data={data}
              margin={{ top: 4, right: 0, bottom: 0, left: -20 }}
              barCategoryGap="18%"
              maxBarSize={44}
            >
              <CartesianGrid vertical={false} stroke="var(--border)" strokeOpacity={0.6} />
              <XAxis dataKey="label" tick={TICK} tickLine={false} axisLine={false} minTickGap={28} tickMargin={8} />
              <YAxis allowDecimals={false} tick={TICK} tickLine={false} axisLine={false} width={40} tickCount={4} />
              <Tooltip
                cursor={{ fill: "var(--muted)", opacity: 0.7 }}
                contentStyle={{
                  background: "var(--popover)",
                  border: "1px solid var(--border)",
                  borderRadius: 8,
                  fontSize: 12,
                  boxShadow: "var(--finding-ring)",
                }}
              />
              <Bar dataKey="affected" name="Affected" stackId="traces" fill={AFFECTED} isAnimationActive={false} />
              <Bar
                dataKey="unaffected"
                name="Unaffected"
                stackId="traces"
                fill={UNAFFECTED}
                radius={TOP_RADIUS}
                isAnimationActive={false}
              />
            </BarChart>
          </ResponsiveContainer>
        </div>
      )}
      <div className="flex flex-wrap items-center justify-between gap-2">
        <Legend />
        <p className="text-xs text-muted-foreground">Share of traces sampled by the reporting investigations</p>
      </div>
    </section>
  );
}
