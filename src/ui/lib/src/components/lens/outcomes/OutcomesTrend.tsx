"use client";

import { Bar, BarChart, CartesianGrid, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import type { OutcomeAgentTrendPoint, OutcomeMetric } from "./outcomesModel";

export const OUTCOME_AGENT_COLORS = ["#3b82f6", "#06b6d4", "#6366f1", "#22c55e", "#f59e0b"];
export const OUTCOME_MEASURES: Record<OutcomeMetric, { label: string; title: string; unit: string }> = {
  mergedPRs: { label: "Merged PRs", title: "Merged pull requests per day", unit: "PRs" },
  closedIssues: { label: "Closed issues", title: "Closed issues per day", unit: "issues" },
  costUSD: { label: "Agent spend", title: "Agent spend per day", unit: "USD" },
};
export const outcomeValue = (value: number, measure: OutcomeMetric) =>
  measure === "costUSD"
    ? value.toLocaleString("en-US", { style: "currency", currency: "USD" })
    : value.toLocaleString();
const dateLabel = (date: string) =>
  new Date(date).toLocaleDateString("en-US", { month: "short", day: "numeric", timeZone: "UTC" });

export function OutcomesTrend({
  data,
  agents,
  measure,
}: {
  readonly data: readonly OutcomeAgentTrendPoint[];
  readonly agents: readonly string[];
  readonly measure: OutcomeMetric;
}) {
  const chart = data.map((day) => ({ date: day.date, ...day.values }));
  const totals = agents.map((agent) => ({
    agent,
    total: data.reduce((sum, day) => sum + (day.values[agent] ?? 0), 0),
  }));
  const total = totals.reduce((sum, item) => sum + item.total, 0);
  const selected = OUTCOME_MEASURES[measure];
  return (
    <section aria-label="Shipped work over time" className="rounded-lg border bg-card p-4 sm:p-5">
      <div className="mb-5 flex flex-wrap items-start justify-between gap-3">
        <h3 className="text-lg font-medium">{selected.title}</h3>
        <div className="flex flex-wrap gap-x-4 gap-y-2 text-xs text-muted-foreground">
          {agents.map((agent, index) => (
            <span key={agent} className="flex items-center gap-1.5">
              <span
                aria-hidden="true"
                className="size-2 rounded-full"
                style={{ background: OUTCOME_AGENT_COLORS[index % OUTCOME_AGENT_COLORS.length] }}
              />
              {agent}
            </span>
          ))}
        </div>
      </div>
      <div
        role="img"
        aria-label={`${selected.title}: ${outcomeValue(total, measure)} ${selected.unit} across ${data.length} daily buckets, segmented by agent`}
        className="h-[220px] w-full"
      >
        <ResponsiveContainer width="100%" height="100%">
          <BarChart data={chart} margin={{ top: 5, right: 8, bottom: 4, left: 0 }} accessibilityLayer={false}>
            <CartesianGrid vertical={false} stroke="var(--border)" />
            <XAxis
              dataKey="date"
              tickFormatter={dateLabel}
              axisLine={false}
              tickLine={false}
              minTickGap={32}
              tick={{ fill: "var(--muted-foreground)", fontSize: 11 }}
            />
            <YAxis
              domain={[0, "dataMax"]}
              allowDecimals={measure === "costUSD"}
              width={52}
              tickFormatter={(value: number) =>
                measure === "costUSD" ? `$${value}` : value.toLocaleString()
              }
              axisLine={false}
              tickLine={false}
              tick={{ fill: "var(--muted-foreground)", fontSize: 11 }}
            />
            <Tooltip
              labelFormatter={(label) => dateLabel(String(label))}
              formatter={(value) => outcomeValue(Number(value), measure)}
              cursor={{ fill: "var(--muted)", opacity: 0.4 }}
              contentStyle={{
                background: "var(--popover)",
                border: "1px solid var(--border)",
                borderRadius: 6,
                fontSize: 12,
              }}
            />
            {agents.map((agent, index) => (
              <Bar
                key={agent}
                dataKey={agent}
                name={agent}
                stackId="agents"
                fill={OUTCOME_AGENT_COLORS[index % OUTCOME_AGENT_COLORS.length]}
                maxBarSize={26}
                isAnimationActive={false}
              />
            ))}
          </BarChart>
        </ResponsiveContainer>
      </div>
      <details className="mt-3 text-xs text-muted-foreground">
        <summary className="w-fit cursor-pointer rounded-sm outline-none focus-visible:ring-2 focus-visible:ring-ring">
          View daily counts
        </summary>
        <div className="mt-3 max-h-60 overflow-auto">
          <table className="w-full text-left tabular-nums">
            <caption className="sr-only">Daily {selected.label.toLowerCase()} by agent</caption>
            <thead>
              <tr>
                <th scope="col" className="py-2 font-normal">
                  Date
                </th>
                {agents.map((agent) => (
                  <th key={agent} scope="col" className="py-2 text-right font-normal">
                    {agent}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {data.map((day) => (
                <tr key={day.date} className="border-t">
                  <th scope="row" className="py-2 font-normal">
                    {dateLabel(day.date)}
                  </th>
                  {agents.map((agent) => (
                    <td key={agent} className="py-2 text-right">
                      {outcomeValue(day.values[agent] ?? 0, measure)}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </details>
    </section>
  );
}
