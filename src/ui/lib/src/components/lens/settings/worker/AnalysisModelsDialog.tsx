"use client";

import { useMemo, useState } from "react";
import { Button } from "../../../ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "../../../ui/dialog";
import { Input } from "../../../ui/input";
import { filterAnalysisModels, sortAnalysisModels } from "../../model/analysisModels";
import type { AnalysisModelInfo } from "../../model/types";

export function AnalysisModelsDialog({ models }: { models: readonly AnalysisModelInfo[] }) {
  const [search, setSearch] = useState("");
  const sorted = useMemo(() => sortAnalysisModels(models), [models]);
  const matching = useMemo(() => filterAnalysisModels(sorted, search), [sorted, search]);

  return (
    <Dialog
      onOpenChange={(open) => {
        if (open) setSearch("");
      }}
    >
      <DialogTrigger render={<Button variant="outline" size="sm" />}>Browse models</DialogTrigger>
      <DialogContent className="flex max-h-[calc(100dvh-2rem)] flex-col gap-4 sm:max-w-xl">
        <DialogHeader className="shrink-0 pr-8">
          <DialogTitle>Analysis models</DialogTitle>
          <DialogDescription>
            Models available for analysis. Choose a model and spending limit in Findings.
          </DialogDescription>
        </DialogHeader>
        <Input
          type="search"
          aria-label="Search models"
          placeholder="Search models…"
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          className="shrink-0"
        />
        <p role="status" className="shrink-0 text-xs text-muted-foreground">
          {matching.length} of {models.length} {models.length === 1 ? "model" : "models"}
        </p>
        <div
          role="region"
          aria-label="Model results"
          tabIndex={matching.length > 0 ? 0 : undefined}
          className="h-80 min-h-0 overflow-y-auto overscroll-contain rounded-md border outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          {matching.length > 0 ? (
            <ul aria-label="Analysis models" className="divide-y font-mono text-xs">
              {matching.map((model) => (
                <li key={model.model_group} className="break-all px-3 py-2.5">
                  {model.model_group}
                </li>
              ))}
            </ul>
          ) : (
            <p className="flex h-full items-center justify-center px-3 text-sm text-muted-foreground">
              No models match your search
            </p>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
