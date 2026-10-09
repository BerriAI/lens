"use client";

import type { ComponentProps } from "react";
import { useLensRoute } from "../route";
import { cn } from "../../../lib/cva.config";

export type ReadinessBannerProps = ComponentProps<"div"> & {
  activityReady: boolean;
};

export function ReadinessBanner({ activityReady, className, ...props }: ReadinessBannerProps) {
  const { setTab } = useLensRoute();
  return (
    <div
      {...props}
      data-slot="readiness-banner"
      role="status"
      className={cn("flex flex-wrap items-center gap-2 border-y py-3 text-sm text-muted-foreground", className)}
    >
      {activityReady
        ? "Configure analysis in Settings to run new investigations. Saved results are still available."
        : "Recorded activity is not ready. Saved results are still available."}
      {!activityReady && (
        <button type="button" className="font-medium underline" onClick={() => setTab("traces")}>
          Check traces
        </button>
      )}
    </div>
  );
}
