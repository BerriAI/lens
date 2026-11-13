import type { Optimization } from "./model";

function average(values: number[]) {
  return values.length ? values.reduce((total, value) => total + value, 0) / values.length : Number.NaN;
}

function uncertainty(values: number[]) {
  if (values.length < 30) return Number.POSITIVE_INFINITY;
  const mean = average(values);
  const variance = values.reduce((total, value) => total + (value - mean) ** 2, 0) / (values.length - 1);
  return 1.96 * Math.sqrt(variance / values.length);
}

export function measuredRows(run: Optimization) {
  const results = run.measured ?? [];
  const baselineId = run.config.baseline ?? "";
  const baseline = results.filter((result) => result.model === baselineId);
  const baselineScores = baseline.flatMap((result) => result.score === null ? [] : [result.score]);
  const baselineCost = baseline.every((result) => result.costUsd !== null) ? average(baseline.flatMap((result) => result.costUsd === null ? [] : [result.costUsd])) : Number.NaN;
  const baselineQuality = average(baselineScores);
  return [baselineId, ...run.config.models].map((id) => {
    const samples = results.filter((result) => result.model === id);
    const scores = samples.flatMap((result) => result.score === null ? [] : [result.score]);
    const quality = average(scores);
    const cost = samples.every((result) => result.costUsd !== null) ? average(samples.flatMap((result) => result.costUsd === null ? [] : [result.costUsd])) : Number.NaN;
    const differences = samples.flatMap((result) => {
      const reference = baseline.find((item) => item.sourceId === result.sourceId);
      return result.score != null && reference?.score != null ? [result.score - reference.score] : [];
    });
    const lowerDifference = run.config.mode === "live"
      ? quality - baselineQuality - Math.hypot(uncertainty(scores), uncertainty(baselineScores))
      : average(differences) - uncertainty(differences);
    const coverage = samples.length ? scores.length / samples.length : 0;
    const baselineCoverage = baseline.length ? baselineScores.length / baseline.length : 0;
    const feedbackCoverage = run.config.judge !== "feedback" || (coverage >= 0.5 && Math.abs(coverage - baselineCoverage) <= 0.1);
    const errors = [...samples, ...baseline].filter((result) => result.error !== null || result.scoreError !== null).length;
    return {
      id, isBaseline: id === baselineId, count: samples.length, scored: scores.length, quality, cost,
      latency: average(samples.flatMap((result) => result.durationMs === null ? [] : [result.durationMs / 1000])),
      savings: baselineCost > 0 && Number.isFinite(cost) ? Math.round((1 - cost / baselineCost) * 100) : Number.NaN,
      qualifies: feedbackCoverage && errors === 0 && Number.isFinite(cost) && lowerDifference >= -run.config.tolerance,
    };
  });
}
