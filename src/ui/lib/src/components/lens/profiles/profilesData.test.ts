import { describe, expect, it } from "vitest";
import {
  cellReading,
  compatibilityNote,
  formatLatency,
  formatUsd,
  heatIntensity,
  MUTED,
  profileColors,
  selectedProfileId,
} from "./profilesData";
import type { MatrixCell } from "./schemas";

const cell = (overrides: Partial<MatrixCell>): MatrixCell => ({
  profile_id: "p",
  column_key: "id:dep",
  request_count: 4,
  avg_cost_usd: null,
  cost_observations: 0,
  p95_latency_ms: null,
  latency_observations: 0,
  eval_success_rate: null,
  eval_sample_count: 0,
  ...overrides,
});

describe("selectedProfileId", () => {
  const profiles = [
    { profile_id: "recorded:Coding", name: "Coding" },
    { profile_id: "task:abc", name: "Debug failing tests" },
  ];

  it("accepts a profile id", () => {
    expect(selectedProfileId(profiles, "task:abc")).toBe("task:abc");
  });

  it("translates an older label link to the profile id", () => {
    expect(selectedProfileId(profiles, "Coding")).toBe("recorded:Coding");
  });

  it("ignores a label this agent does not have", () => {
    expect(selectedProfileId(profiles, "Workflows")).toBeNull();
  });
});

describe("profileColors", () => {
  it("keeps placeholder profiles muted without spending palette colors on them", () => {
    const colors = profileColors([
      { profile_id: "pending", source: "pending" },
      { profile_id: "task:a", source: "enhanced" },
      { profile_id: "unclassified", source: "unclassified" },
      { profile_id: "task:b", source: "enhanced" },
    ]);

    expect(colors.get("pending")).toBe(MUTED);
    expect(colors.get("unclassified")).toBe(MUTED);
    expect(colors.get("task:a")).toBe("var(--chart-1)");
    expect(colors.get("task:b")).toBe("var(--chart-2)");
  });
});

describe("cellReading", () => {
  it("reports missing cost as no value with its observation count", () => {
    expect(cellReading(cell({ avg_cost_usd: null, cost_observations: 0 }), "cost")).toEqual({ value: null, observations: 0 });
  });

  it("returns no evidence for a model that never served the profile", () => {
    expect(cellReading(undefined, "requests")).toEqual({ value: null, observations: 0 });
  });
});

describe("formatting", () => {
  it("never shows a missing cost or latency as zero", () => {
    expect(formatUsd(null)).toBeNull();
    expect(formatLatency(null)).toBeNull();
    expect(formatUsd(0)).toBe("$0.00");
  });

  it("keeps sub-cent costs readable", () => {
    expect(formatUsd(0.0042)).toBe("$0.0042");
  });

  it("switches latency to seconds above one second", () => {
    expect(formatLatency(1500)).toBe("1.5 s");
    expect(formatLatency(320.4)).toBe("320 ms");
  });
});

describe("heatIntensity", () => {
  it("spans the observed range and leaves missing cells unshaded", () => {
    const values = [2, null, 6];

    expect(heatIntensity(2, values)).toBe(0);
    expect(heatIntensity(6, values)).toBe(1);
    expect(heatIntensity(null, values)).toBeNull();
  });
});

describe("compatibilityNote", () => {
  it.each([
    { name: "unreviewed profile", requirements: null, fit: undefined, note: "Requirements not reviewed" },
    { name: "met requirements", requirements: ["vision"], fit: { status: "compatible", missing: [], unknown: [] }, note: "Meets declared requirements" },
    { name: "no requirements", requirements: [], fit: { status: "compatible", missing: [], unknown: [] }, note: "No special requirements" },
    { name: "missing capability", requirements: ["vision"], fit: { status: "incompatible", missing: ["vision"], unknown: [] }, note: "Metadata says it lacks vision" },
    { name: "undeclared capability", requirements: ["function_calling"], fit: { status: "unknown", missing: [], unknown: ["function_calling"] }, note: "function calling support unknown" },
  ] as const)("describes a $name", ({ requirements, fit, note }) => {
    expect(compatibilityNote(requirements, fit)).toBe(note);
  });
});
