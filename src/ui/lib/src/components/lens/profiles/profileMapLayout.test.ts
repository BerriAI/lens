import { describe, expect, it } from "vitest";
import {
  type Box,
  estimateWidth,
  fitToPlot,
  LABEL_FONT,
  layoutMap,
  type MapGroupInput,
  type Size,
  wrapLabel,
} from "./profileMapLayout";

const group = (profileId: string, count: number, at: { x: number; y: number }, name = profileId): MapGroupInput => ({
  profileId,
  name,
  count,
  points: Array.from({ length: count }, (_, index) => ({
    key: `${profileId}:${index}`,
    x: at.x + (index % 3) * 0.4,
    y: at.y + Math.floor(index / 3) * 0.4,
  })),
});

const crowded = Array.from({ length: 12 }, (_, index) =>
  group(`task:${index}`, 12 - index, { x: 50 + (index % 2), y: 50 + (index % 3) }, `Investigate failing builds ${index}`),
);

const overlapping = (a: Box, b: Box) => a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;

const within = (box: Box, size: Size) =>
  box.x >= 0 && box.y >= 0 && box.x + box.width <= size.width && box.y + box.height <= size.height;

describe("fitToPlot", () => {
  it("scales positions uniformly so relative distances survive", () => {
    const size = { width: 600, height: 320 };
    const place = fitToPlot([{ x: 0, y: 0 }, { x: 10, y: 0 }, { x: 0, y: 5 }], size);
    const [origin, right, down] = [place({ x: 0, y: 0 }), place({ x: 10, y: 0 }), place({ x: 0, y: 5 })];

    expect((right.x - origin.x) / (down.y - origin.y)).toBeCloseTo(2);
    expect([origin, right, down].every((point) => point.x >= 0 && point.x <= 600 && point.y >= 0 && point.y <= 320)).toBe(true);
  });

  it("centers a single position instead of collapsing it to a corner", () => {
    expect(fitToPlot([{ x: 7, y: 7 }], { width: 400, height: 300 })({ x: 7, y: 7 })).toEqual({ x: 200, y: 150 });
  });
});

describe("wrapLabel", () => {
  it("wraps a long task name to two lines that fit and marks the cut", () => {
    const lines = wrapLabel("Troubleshoot broken features after the latest mobile release rollout", 120);

    expect(lines).toHaveLength(2);
    expect(lines.every((line) => estimateWidth(line, "name") <= 120)).toBe(true);
    expect(lines[1].endsWith("…")).toBe(true);
  });

  it("breaks a single word wider than the label", () => {
    const lines = wrapLabel("supercalifragilistic", 60);

    expect(lines.every((line) => estimateWidth(line, "name") <= 60)).toBe(true);
    expect(lines[0]).toBe("supercal");
  });
});

describe("layoutMap", () => {
  it.each([
    { width: 320, height: 260 },
    { width: 640, height: 380 },
  ])("keeps every label inside a $width px plot without overlaps", (size) => {
    const layout = layoutMap(crowded, size);

    expect(layout.labels.length).toBeGreaterThan(0);
    expect(layout.labels.every((label) => within(label.box, size))).toBe(true);
    const clashes = layout.labels.flatMap((label, index) =>
      layout.labels.slice(index + 1).filter((other) => overlapping(label.box, other.box)),
    );
    expect(clashes).toEqual([]);
    expect([...layout.labels.map((label) => label.profileId), ...layout.hidden].sort()).toEqual(
      crowded.map((entry) => entry.profileId).sort(),
    );
  });

  it("sizes each label to hold its measured lines", () => {
    const layout = layoutMap(crowded, { width: 640, height: 380 });

    expect(
      layout.labels.every((label) =>
        label.lines.every((line) => estimateWidth(line, "name") <= label.box.width - 2 * LABEL_FONT.padding),
      ),
    ).toBe(true);
  });

  it("labels the selected small group first when space runs out", () => {
    const size = { width: 160, height: 140 };
    const small = "task:11";

    expect(layoutMap(crowded, size).hidden).toContain(small);
    expect(layoutMap(crowded, size, { priority: small }).labels.map((label) => label.profileId)).toContain(small);
  });

  it("places labels beside clusters instead of covering their points", () => {
    const layout = layoutMap([group("task:a", 3, { x: 0, y: 0 }), group("task:b", 3, { x: 100, y: 0 })], {
      width: 600,
      height: 320,
    });
    const points = layout.groups.flatMap((entry) => entry.points);
    const covered = layout.labels.filter((label) =>
      points.some(
        (point) =>
          point.x >= label.box.x &&
          point.x <= label.box.x + label.box.width &&
          point.y >= label.box.y &&
          point.y <= label.box.y + label.box.height,
      ),
    );

    expect(layout.labels).toHaveLength(2);
    expect(covered).toEqual([]);
  });

  it("zooms to the selected group so its points spread across the plot", () => {
    const groups = [group("task:a", 6, { x: 0, y: 0 }), group("task:b", 6, { x: 100, y: 100 })];
    const size = { width: 600, height: 320 };
    const spread = (zoom: string | null) => {
      const points = layoutMap(groups, size, { zoom }).groups.find((entry) => entry.profileId === "task:a")?.points ?? [];
      return Math.max(...points.map((point) => point.x)) - Math.min(...points.map((point) => point.x));
    };

    expect(spread("task:a")).toBeGreaterThan(spread(null) * 3);
    expect(layoutMap(groups, size, { zoom: "task:a" }).labels.map((label) => label.profileId)).toEqual(["task:a"]);
  });

  it("draws nothing before the plot has a size", () => {
    expect(layoutMap(crowded, { width: 0, height: 300 })).toEqual({ groups: [], labels: [], hidden: [] });
  });
});
