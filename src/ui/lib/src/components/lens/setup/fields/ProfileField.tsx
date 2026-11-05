"use client";

import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Controller, useFormContext, useWatch } from "react-hook-form";
import { useProfilesApi } from "../../data/LensServices";
import type { InvestigationInput } from "../investigationSchema";
import { useDebouncedValue } from "../useDebouncedValue";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../../ui/select";

export function ProfileField() {
  const api = useProfilesApi();
  const { control } = useFormContext<InvestigationInput>();
  const selection = useWatch({ control, name: "selection" });
  const [endMs] = useState(Date.now);
  const hours = selection.lookback_hours || 24;
  const { value: scope } = useDebouncedValue({ agent: selection.agent_name, hours }, 350);
  const query = useQuery({
    queryKey: ["lens", "setup-profiles", api.scope, scope],
    queryFn: ({ signal }) =>
      api.usage({ startMs: endMs - scope.hours * 3_600_000, endMs }, scope.agent, signal),
    enabled: selection.source !== "requests" && Number.isInteger(hours) && hours > 0,
    staleTime: 30_000,
  });
  const categories = [
    ...new Set([
      ...(query.data ?? []).map((category) => category.category),
      ...(selection.profile ? [selection.profile] : []),
    ]),
  ];
  const items = [
    { value: "", label: "All profiles" },
    ...categories.map((category) => ({ value: category, label: category })),
  ];
  if (selection.source === "requests") return null;
  return (
    <label className="grid gap-2 text-sm font-medium">
      Profile
      <Controller
        control={control}
        name="selection.profile"
        render={({ field }) => (
          <Select
            items={items}
            value={field.value ?? ""}
            onValueChange={(value: string | null) => field.onChange(value || null)}
          >
            <SelectTrigger aria-label="Profile" className="w-full bg-background">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {items.map((item) => (
                <SelectItem key={item.value} value={item.value}>
                  {item.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        )}
      />
      <span className="text-xs font-normal text-muted-foreground">
        {query.isPending
          ? "Loading task categories…"
          : query.error
            ? "Could not load profiles. Your selected profile is preserved."
            : "Review one task category, such as Coding or Workflows"}
      </span>
    </label>
  );
}
