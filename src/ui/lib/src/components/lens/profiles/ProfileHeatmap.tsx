"use client";

import { useMemo, useState } from "react";
import { cn } from "../../../lib/cva.config";
import {
  cellReading,
  compatibilityNote,
  formatCount,
  formatReading,
  HEAT_METRICS,
  heatIntensity,
  type HeatMetric,
} from "./profilesData";
import type { ProfilesMatrix } from "./schemas";

const METRIC_LABELS: Record<HeatMetric, string> = {
  requests: "Requests",
  cost: "Avg cost",
  latency: "p95 latency",
};

export function ProfileHeatmap({
  matrix,
  selected,
  onSelect,
}: {
  matrix: ProfilesMatrix;
  selected: string | null;
  onSelect: (profileId: string) => void;
}) {
  const [metric, setMetric] = useState<HeatMetric>("requests");
  const cells = useMemo(
    () => new Map(matrix.cells.map((cell) => [`${cell.profile_id}\u0000${cell.column_key}`, cell])),
    [matrix.cells],
  );
  const compatibility = useMemo(
    () => new Map(matrix.compatibility.map((entry) => [`${entry.profile_id}\u0000${entry.column_key}`, entry])),
    [matrix.compatibility],
  );
  const values = matrix.cells.map((cell) => cellReading(cell, metric).value);
  if (matrix.profiles.length === 0) return null;
  return (
    <div className="flex flex-col gap-3">
      <div role="radiogroup" aria-label="Heatmap metric" className="flex w-fit rounded-lg bg-muted p-1">
        {HEAT_METRICS.map((option) => (
          <button
            key={option}
            type="button"
            role="radio"
            aria-checked={metric === option}
            className={cn(
              "rounded-md px-3 py-1 text-xs transition-colors",
              metric === option ? "bg-card text-foreground shadow-xs" : "text-muted-foreground hover:text-foreground",
            )}
            onClick={() => setMetric(option)}
          >
            {METRIC_LABELS[option]}
          </button>
        ))}
      </div>
      <div className="overflow-x-auto">
        <table className="w-full border-separate border-spacing-1 text-xs">
          <caption className="sr-only">Requests, average cost and p95 latency per profile and model deployment</caption>
          <thead>
            <tr>
              <th scope="col" className="text-left font-medium text-muted-foreground">
                Profile
              </th>
              {matrix.models.map((model) => (
                <th key={model.column_key} scope="col" className="min-w-28 text-left align-bottom font-medium">
                  <span className="block break-words">{model.model_name || "Unknown model"}</span>
                  <span className="block font-normal text-muted-foreground">
                    {model.model_config_id ?? "Deployment ID not recorded"}
                    {model.configured && !matrix.cells.some((cell) => cell.column_key === model.column_key) && " · no traffic"}
                  </span>
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {matrix.profiles.map((profile) => (
              <tr key={profile.profile_id}>
                <th scope="row" className="max-w-48 text-left font-medium break-words">
                  {profile.name}
                </th>
                {matrix.models.map((model) => {
                  const key = `${profile.profile_id}\u0000${model.column_key}`;
                  const reading = cellReading(cells.get(key), metric);
                  const label = formatReading(metric, reading.value) ?? "No data";
                  const intensity = heatIntensity(reading.value, values);
                  const fit = compatibility.get(key);
                  return (
                    <td key={model.column_key} className="p-0">
                      <button
                        type="button"
                        aria-label={`${profile.name} on ${model.model_name || "unknown model"}: ${label}`}
                        aria-pressed={selected === profile.profile_id}
                        onClick={() => onSelect(profile.profile_id)}
                        className={cn(
                          "flex h-full w-full flex-col items-start gap-0.5 rounded-md border px-2 py-1.5 text-left",
                          intensity === null && "bg-muted/40 text-muted-foreground",
                        )}
                        style={
                          intensity === null
                            ? undefined
                            : { backgroundColor: `color-mix(in oklab, var(--chart-1) ${Math.round(12 + intensity * 55)}%, transparent)` }
                        }
                      >
                        <span className="font-medium tabular-nums">{label}</span>
                        {metric !== "requests" && reading.observations > 0 && (
                          <span className="text-[11px] text-muted-foreground">{formatCount(reading.observations)} observed</span>
                        )}
                        <span
                          className={cn(
                            "text-[11px]",
                            fit?.status === "incompatible" ? "text-destructive" : "text-muted-foreground",
                          )}
                        >
                          {compatibilityNote(profile.required_capabilities, fit)}
                        </span>
                      </button>
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <p className="text-xs text-muted-foreground">
        Compatibility compares a reviewed task&apos;s requirements with the gateway&apos;s model metadata. It is not a
        quality score. Evaluation results are not collected yet.
        {matrix.other_agent_requests > 0 &&
          ` ${formatCount(matrix.other_agent_requests)} requests made by other agents in these runs are not counted.`}
      </p>
    </div>
  );
}
