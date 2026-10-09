import type { Gate, Scorer } from "./runs/types";

export function scorerLabel(scorer: Scorer): string {
  switch (scorer.kind) {
    case "task_completed":
      return "task_completed";
    case "called_before":
      return `${scorer.first} before ${scorer.then}`;
    case "judge":
      return "judge";
  }
}

export function gateLabel(gate: Gate): string {
  const parts = [
    gate.regressions != null && `regressions ≤ ${gate.regressions}`,
    gate.critical != null && `critical ≤ ${gate.critical}`,
    gate.pass_rate != null && `pass ≥ ${Math.round(gate.pass_rate * 100)}%`,
    gate.cost_per_case != null && `$${gate.cost_per_case}/case`,
  ].filter((part): part is string => typeof part === "string");
  return parts.length ? parts.join(" · ") : "no gate";
}
