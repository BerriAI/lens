import { ExternalLink } from "lucide-react";
import type { OptimizationConfig } from "./model";

export const optimizationGuideUrl = "https://github.com/BerriAI/lens/blob/main/docs/model-optimizations.md";

const guides = {
  live: {
    description: "Try candidate models on a share of your live agent runs. Compare user feedback or judge scores with cost and latency.",
    label: "Live",
    anchor: "live-example",
  },
  shadow: {
    description: "Replay the final model call from sampled traces with candidate models. Your users keep receiving the original answer.",
    label: "Shadow",
    anchor: "shadow-example",
  },
  offline: {
    description: "Compare models on a saved dataset. Replay the final model call or run your full agent for each case and model.",
    label: "Offline",
    anchor: "offline-example",
  },
};

export function OptimizationGuide({ mode, scope }: Pick<OptimizationConfig, "mode" | "scope">) {
  const guide = mode === "offline" && scope === "agent"
    ? {
        description: "Run your agent on each dataset case with each model. Your SDK callback runs the tools and model calls; Lens compares the completed traces.",
        label: "full-agent",
        anchor: "full-agent-example",
      }
    : guides[mode];
  return (
    <div className="grid gap-2 text-sm text-muted-foreground">
      <p>{guide.description}</p>
      <a
        href={`${optimizationGuideUrl}#${guide.anchor}`}
        target="_blank"
        rel="noopener noreferrer"
        className="inline-flex w-fit items-center gap-1.5 text-primary underline-offset-4 hover:underline"
      >
        View {guide.label} example
        <ExternalLink className="size-3.5" aria-hidden="true" />
      </a>
    </div>
  );
}
