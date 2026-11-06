"use client";

import { useState } from "react";
import { CalendarDays } from "lucide-react";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "../../ui/popover";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "../../ui/select";
import { useOutcomesRoute } from "../route";
import { CODING_AGENTS, outcomeDateStart } from "./outcomesModel";

const PERIODS = [
  { value: "7", label: "Last 7 days" },
  { value: "30", label: "Last 30 days" },
  { value: "90", label: "Last 90 days" },
  { value: "custom", label: "Custom range" },
] as const;
const dateLabel = (value: string) =>
  new Date(`${value}T00:00:00Z`).toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    timeZone: "UTC",
  });

export function CodingOutcomeFilters({
  teams,
  now,
}: {
  teams: readonly string[];
  now: number;
}) {
  const filters = useOutcomesRoute();
  const [open, setOpen] = useState(false);
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const teamOptions = [
    { value: "all", label: "All teams" },
    ...teams.map((team) => ({ value: team, label: team })),
  ];
  const agentOptions = [
    { value: "all", label: "All coding agents" },
    ...CODING_AGENTS,
  ];
  const start = outcomeDateStart(from);
  const end = outcomeDateStart(to);
  const valid = start !== null && end !== null && start <= end;
  const hasRange = filters.from !== null || filters.to !== null;
  const onOpenChange = (next: boolean) => {
    if (next) {
      setFrom(
        filters.from ??
          new Date(now - Number(filters.days) * 86_400_000)
            .toISOString()
            .slice(0, 10),
      );
      setTo(filters.to ?? new Date(now).toISOString().slice(0, 10));
    }
    setOpen(next);
  };
  return (
    <div className="flex flex-wrap items-center gap-2">
      <Select
        items={teamOptions}
        value={filters.team}
        onValueChange={(value: string | null) =>
          value && filters.setTeam(value)
        }
      >
        <SelectTrigger aria-label="Team" className="h-8 min-w-36 text-xs">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {teamOptions.map((item) => (
            <SelectItem key={item.value} value={item.value}>
              {item.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <Select
        items={agentOptions}
        value={filters.codingAgent}
        onValueChange={(value: typeof filters.codingAgent | null) =>
          value && filters.setCodingAgent(value)
        }
      >
        <SelectTrigger
          aria-label="Coding agent"
          className="h-8 min-w-40 text-xs"
        >
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {agentOptions.map((item) => (
            <SelectItem key={item.value} value={item.value}>
              {item.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <div className="flex items-center gap-1">
        <Select<(typeof PERIODS)[number]["value"]>
          items={PERIODS}
          value={hasRange ? "custom" : filters.days}
          onValueChange={(value: "7" | "30" | "90" | "custom" | null) => {
            if (value === "custom") onOpenChange(true);
            else if (value) filters.setDays(value);
          }}
        >
          <SelectTrigger
            aria-label="Time range"
            className="h-8 min-w-32 text-xs"
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {PERIODS.map((item) => (
              <SelectItem key={item.value} value={item.value}>
                {item.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Popover open={open} onOpenChange={onOpenChange}>
          <PopoverTrigger
            render={
              <Button
                variant="outline"
                size="sm"
                className="h-8 text-xs"
                aria-label="Custom date range"
              />
            }
          >
            <CalendarDays className="size-3.5" />
            {filters.from && filters.to
              ? `${dateLabel(filters.from)} - ${dateLabel(filters.to)}`
              : "Date range"}
          </PopoverTrigger>
          <PopoverContent
            role="dialog"
            aria-label="Custom date range"
            align="end"
            className="w-80 gap-3"
          >
            <p className="text-sm font-medium">Custom date range</p>
            <div className="grid grid-cols-2 gap-3">
              <label className="grid gap-1.5 text-xs">
                From date
                <Input
                  type="date"
                  aria-label="From date"
                  value={from}
                  max={to || undefined}
                  onChange={(event) => setFrom(event.target.value)}
                />
              </label>
              <label className="grid gap-1.5 text-xs">
                To date
                <Input
                  type="date"
                  aria-label="To date"
                  value={to}
                  min={from || undefined}
                  onChange={(event) => setTo(event.target.value)}
                />
              </label>
            </div>
            {start !== null && end !== null && start > end && (
              <p role="alert" className="text-xs text-destructive">
                End date must follow the start date
              </p>
            )}
            <p className="text-[11px] text-muted-foreground">
              Dates select session starts and include the full day in UTC
            </p>
            <Button
              size="sm"
              disabled={!valid}
              onClick={() => {
                filters.setDateRange(from, to);
                setOpen(false);
              }}
            >
              Apply range
            </Button>
          </PopoverContent>
        </Popover>
      </div>
    </div>
  );
}
