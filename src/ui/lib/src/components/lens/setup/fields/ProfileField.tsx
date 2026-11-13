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
    queryFn: async ({ signal }) => {
      const window = { startMs: endMs - scope.hours * 3_600_000, endMs };
      if (!scope.agent) {
        const categories = await api.usage(window, scope.agent, signal);
        return categories.map((category) => ({ value: category.category, label: category.category }));
      }
      const overview = await api.overview(window, scope.agent, signal);
      return overview.profiles.map((profile) => ({ value: profile.profile_id, label: profile.name }));
    },
    enabled: selection.source !== "requests" && Number.isInteger(hours) && hours > 0,
    staleTime: 30_000,
  });
  const options = query.data ?? [];
  const items = [
    { value: "", label: "All profiles" },
    ...options,
    ...(selection.profile && !options.some((option) => option.value === selection.profile)
      ? [{ value: selection.profile, label: selection.profile }]
      : []),
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
