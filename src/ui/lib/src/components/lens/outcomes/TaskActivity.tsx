"use client";

import type { ReactNode } from "react";
import {
  Bar,
  BarChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "../../ui/table";
import type { TaskActivityPoint, TaskKindSummary } from "./outcomesModel";

const COLORS = [
  "#3b82f6",
  "#06b6d4",
  "#6366f1",
  "#8b5cf6",
  "#93c5fd",
  "#94a3b8",
];
const TASK_COLORS: ReadonlyMap<string, string> = new Map([
  ["Bug fixes", COLORS[0]],
  ["Features", COLORS[1]],
  ["Testing", COLORS[2]],
  ["Code review", COLORS[3]],
  ["Docs & research", COLORS[4]],
]);

export function taskColor(kind: string): string {
  const knownColor = TASK_COLORS.get(kind);
  if (knownColor) return knownColor;
  const hash = Array.from(kind).reduce(
    (value, character) => (value * 31 + character.charCodeAt(0)) >>> 0,
    0,
  );
  return COLORS[hash % COLORS.length];
}

const TICK = { fontSize: 10, fill: "var(--muted-foreground)" };
const AXIS_DATE = new Intl.DateTimeFormat("en-US", {
  month: "short",
  day: "numeric",
  timeZone: "UTC",
});
const FULL_DATE = new Intl.DateTimeFormat("en-US", {
  dateStyle: "medium",
  timeZone: "UTC",
});
const percent = (value: number) => `${Math.round(value * 100)}%`;
const rangeLabel = (point: TaskActivityPoint) => {
  const start = FULL_DATE.format(new Date(point.date));
  return point.date === point.endDate
    ? start
    : `${start} to ${FULL_DATE.format(new Date(point.endDate))}`;
};

export function TaskActivity({
  data,
  categories,
}: {
  data: readonly TaskActivityPoint[];
  categories: readonly TaskKindSummary[];
}) {
  const total = categories.reduce(
    (sum, category) => sum + category.sessions,
    0,
  );
  const description = categories
    .map(
      (category) =>
        `${category.kind}: ${category.sessions.toLocaleString()} sessions, ${percent(category.sessionShare)}`,
    )
    .join("; ");

  return (
    <section aria-label="Task activity" className="min-w-0">
      <div className="mb-3 flex items-baseline justify-between gap-3">
        <h3 className="text-sm font-medium">Task activity</h3>
        <span className="text-[11px] tabular-nums text-muted-foreground">
          {total.toLocaleString()} sessions
        </span>
      </div>
      {total === 0 ? (
        <p className="py-8 text-xs text-muted-foreground">
          No task activity in this period
        </p>
      ) : (
        <>
          <div className="grid min-w-0 items-center gap-x-6 gap-y-3 sm:grid-cols-[minmax(0,1fr)_13rem]">
            <div
              role="img"
              aria-label={`Task activity stacked bar chart: ${total.toLocaleString()} sessions. ${description}`}
              className="h-[170px] min-w-0"
            >
              <ResponsiveContainer width="100%" height="100%">
                <BarChart
                  data={data}
                  accessibilityLayer={false}
                  margin={{ top: 4, right: 0, bottom: 0, left: 0 }}
                  barCategoryGap="24%"
                  maxBarSize={18}
                >
                  <CartesianGrid
                    vertical={false}
                    stroke="var(--border)"
                    strokeOpacity={0.6}
                  />
                  <XAxis
                    dataKey="date"
                    tick={TICK}
                    tickFormatter={(value: string) =>
                      AXIS_DATE.format(new Date(value))
                    }
                    tickLine={false}
                    axisLine={false}
                    minTickGap={30}
                    tickMargin={8}
                  />
                  <YAxis
                    allowDecimals={false}
                    tick={TICK}
                    tickLine={false}
                    axisLine={false}
                    width={28}
                    tickCount={4}
                  />
                  <Tooltip
                    cursor={{ fill: "var(--muted)", opacity: 0.6 }}
                    isAnimationActive={false}
                    labelFormatter={(label: ReactNode) => {
                      const point = data.find((item) => item.date === label);
                      return point ? rangeLabel(point) : label;
                    }}
                    formatter={(value, name) => [
                      typeof value === "number"
                        ? `${value.toLocaleString()} sessions`
                        : value,
                      name,
                    ]}
                    contentStyle={{
                      background: "var(--popover)",
                      border: "1px solid var(--border)",
                      borderRadius: 6,
                      fontSize: 11,
                    }}
                  />
                  {categories.map((category) => (
                    <Bar
                      key={category.kind}
                      dataKey={(point: TaskActivityPoint) =>
                        point.counts[category.kind] ?? 0
                      }
                      name={category.kind}
                      stackId="sessions"
                      fill={taskColor(category.kind)}
                      isAnimationActive={false}
                    />
                  ))}
                </BarChart>
              </ResponsiveContainer>
            </div>
            <ul
              aria-label="Task categories"
              className="min-w-0 space-y-2 text-[11px]"
            >
              {categories.map((category) => (
                <li
                  key={category.kind}
                  className="flex min-w-0 items-center gap-2"
                >
                  <span
                    aria-hidden="true"
                    className="size-2 shrink-0 rounded-sm"
                    style={{ background: taskColor(category.kind) }}
                  />
                  <span
                    className="min-w-0 flex-1 truncate"
                    title={category.kind}
                  >
                    {category.kind}
                  </span>
                  <span className="tabular-nums">
                    {category.sessions.toLocaleString()}
                    <span className="sr-only"> sessions</span>
                  </span>
                  <span className="w-9 text-right tabular-nums text-muted-foreground">
                    {percent(category.sessionShare)}
                  </span>
                </li>
              ))}
            </ul>
          </div>
          <details className="mt-2 text-[11px] text-muted-foreground">
            <summary className="w-fit cursor-pointer rounded-sm focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring">
              Daily data
            </summary>
            <div className="mt-2 overflow-x-auto">
              <Table
                aria-label="Task activity daily data"
                className="text-[11px]"
              >
                <TableHeader>
                  <TableRow>
                    <TableHead scope="col">Date</TableHead>
                    {categories.map((category) => (
                      <TableHead
                        key={category.kind}
                        scope="col"
                        className="text-right"
                      >
                        {category.kind}
                      </TableHead>
                    ))}
                    <TableHead scope="col" className="text-right">
                      Sessions
                    </TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {data.map((point) => (
                    <TableRow key={point.date}>
                      <TableCell className="whitespace-nowrap">
                        {rangeLabel(point)}
                      </TableCell>
                      {categories.map((category) => (
                        <TableCell
                          key={category.kind}
                          className="text-right tabular-nums"
                        >
                          {(point.counts[category.kind] ?? 0).toLocaleString()}
                        </TableCell>
                      ))}
                      <TableCell className="text-right tabular-nums">
                        {point.sessions.toLocaleString()}
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
          </details>
        </>
      )}
    </section>
  );
}
