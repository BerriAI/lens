export interface Position {
  x: number;
  y: number;
}

export interface Size {
  width: number;
  height: number;
}

export interface Box extends Position, Size {}

export interface MapGroupInput {
  profileId: string;
  name: string;
  count: number;
  points: readonly (Position & { key: string })[];
}

export interface PlottedGroup {
  profileId: string;
  points: readonly (Position & { key: string })[];
  centroid: Position;
}

export interface PlacedLabel {
  profileId: string;
  lines: readonly string[];
  countText: string;
  box: Box;
  anchor: Position;
  leader: Position | null;
}

export interface MapLayout {
  groups: readonly PlottedGroup[];
  labels: readonly PlacedLabel[];
  hidden: readonly string[];
}

export type LabelFont = "name" | "count";
export type Measure = (text: string, font: LabelFont) => number;

export const LABEL_FONT = { name: 12, count: 11, nameLine: 15, countLine: 14, padding: 5 } as const;

export const estimateWidth: Measure = (text, font) => text.length * (font === "name" ? 7.4 : 6.6);
const GAP = 3;
const MAX_LINES = 2;
const LEADER_MIN = 10;
const DIRECTIONS: readonly Position[] = [
  { x: 0, y: -1 },
  { x: 0, y: 1 },
  { x: 1, y: 0 },
  { x: -1, y: 0 },
  { x: 1, y: -1 },
  { x: -1, y: -1 },
  { x: 1, y: 1 },
  { x: -1, y: 1 },
];
const RING_STEPS = [6, 26, 56, 96] as const;

export function fitToPlot(points: readonly Position[], size: Size, minSpan = 0): (point: Position) => Position {
  const xs = points.map((point) => point.x);
  const ys = points.map((point) => point.y);
  const measured = { x: Math.max(...xs) - Math.min(...xs), y: Math.max(...ys) - Math.min(...ys) };
  const span = { x: Math.max(measured.x, minSpan), y: Math.max(measured.y, minSpan) };
  const low = { x: Math.min(...xs) - (span.x - measured.x) / 2, y: Math.min(...ys) - (span.y - measured.y) / 2 };
  const padding = { x: Math.min(48, size.width * 0.1), y: Math.min(36, size.height * 0.12) };
  const room = { x: Math.max(size.width - 2 * padding.x, 0), y: Math.max(size.height - 2 * padding.y, 0) };
  const scale = Math.min(span.x > 0 ? room.x / span.x : Infinity, span.y > 0 ? room.y / span.y : Infinity);
  const factor = Number.isFinite(scale) ? scale : 0;
  const offset = { x: (size.width - span.x * factor) / 2, y: (size.height - span.y * factor) / 2 };
  return (point) => ({ x: offset.x + (point.x - low.x) * factor, y: offset.y + (point.y - low.y) * factor });
}

const ZOOM_CONTEXT = 0.2;

export function wrapLabel(name: string, maxWidth: number, measure: Measure = estimateWidth): string[] {
  const fits = (text: string) => measure(text, "name") <= maxWidth;
  const words = name.trim().split(/\s+/).flatMap((word) => split(word, fits));
  const lines = words.reduce<string[]>((built, word) => {
    const last = built.at(-1);
    if (last !== undefined && fits(`${last} ${word}`)) return [...built.slice(0, -1), `${last} ${word}`];
    return [...built, word];
  }, []);
  if (lines.length <= MAX_LINES) return lines;
  return [...lines.slice(0, MAX_LINES - 1), ellipsize(lines.slice(MAX_LINES - 1).join(" "), fits)];
}

function split(word: string, fits: (text: string) => boolean): string[] {
  if (fits(word) || word.length <= 1) return [word];
  const cut = Math.max([...word].findIndex((_, index) => !fits(word.slice(0, index + 1))), 1);
  return [word.slice(0, cut), ...split(word.slice(cut), fits)];
}

function ellipsize(text: string, fits: (text: string) => boolean): string {
  const end = [...text].findIndex((_, index) => !fits(`${text.slice(0, index + 1).trimEnd()}…`));
  return `${text.slice(0, Math.max(end, 1)).trimEnd()}…`;
}

const overlaps = (a: Box, b: Box) =>
  a.x < b.x + b.width + GAP && b.x < a.x + a.width + GAP && a.y < b.y + b.height + GAP && b.y < a.y + a.height + GAP;

const inside = (box: Box, size: Size) =>
  box.x >= 0 && box.y >= 0 && box.x + box.width <= size.width && box.y + box.height <= size.height;

const covers = (box: Box, point: Position) =>
  point.x >= box.x - 2 && point.x <= box.x + box.width + 2 && point.y >= box.y - 2 && point.y <= box.y + box.height + 2;

function candidate(anchor: Position, direction: Position, distance: number, size: Size): Box {
  const diagonal = direction.x !== 0 && direction.y !== 0 ? Math.SQRT1_2 : 1;
  const reach = { x: direction.x * distance * diagonal, y: direction.y * distance * diagonal };
  const x =
    direction.x > 0 ? anchor.x + reach.x : direction.x < 0 ? anchor.x + reach.x - size.width : anchor.x - size.width / 2;
  const y =
    direction.y > 0 ? anchor.y + reach.y : direction.y < 0 ? anchor.y + reach.y - size.height : anchor.y - size.height / 2;
  return { x, y, ...size };
}

function nearestEdge(box: Box, point: Position): Position {
  return {
    x: Math.min(Math.max(point.x, box.x), box.x + box.width),
    y: Math.min(Math.max(point.y, box.y), box.y + box.height),
  };
}

function labelSize(lines: readonly string[], countText: string, measure: Measure): Size {
  const longest = Math.max(...lines.map((line) => measure(line, "name")), measure(countText, "count"));
  return {
    width: Math.ceil(longest + 2 * LABEL_FONT.padding),
    height: lines.length * LABEL_FONT.nameLine + LABEL_FONT.countLine + 2 * LABEL_FONT.padding,
  };
}

export function layoutMap(
  groups: readonly MapGroupInput[],
  size: Size,
  {
    measure = estimateWidth,
    priority = null,
    zoom = null,
  }: { measure?: Measure; priority?: string | null; zoom?: string | null } = {},
): MapLayout {
  const all = groups.flatMap((group) => group.points);
  if (all.length === 0 || size.width <= 0 || size.height <= 0) return { groups: [], labels: [], hidden: [] };
  const focused = groups.find((group) => group.profileId === zoom)?.points ?? [];
  const extent = Math.max(
    Math.max(...all.map((point) => point.x)) - Math.min(...all.map((point) => point.x)),
    Math.max(...all.map((point) => point.y)) - Math.min(...all.map((point) => point.y)),
  );
  const place = focused.length > 0 ? fitToPlot(focused, size, extent * ZOOM_CONTEXT) : fitToPlot(all, size);
  const plotted: PlottedGroup[] = groups
    .filter((group) => group.points.length > 0)
    .map((group) => {
      const points = group.points.map((point) => ({ key: point.key, ...place(point) }));
      return {
        profileId: group.profileId,
        points,
        centroid: {
          x: points.reduce((sum, point) => sum + point.x, 0) / points.length,
          y: points.reduce((sum, point) => sum + point.y, 0) / points.length,
        },
      };
    });
  const positions = plotted.flatMap((group) => group.points);
  const maxWidth = Math.min(176, size.width * 0.42) - 2 * LABEL_FONT.padding;
  const visible = (point: Position) => point.x >= 0 && point.y >= 0 && point.x <= size.width && point.y <= size.height;
  const order = [...groups]
    .filter((group) => group.points.length > 0)
    .filter((group) => {
      const target = plotted.find((entry) => entry.profileId === group.profileId);
      return target !== undefined && visible(target.centroid);
    })
    .sort(
      (a, b) =>
        Number(b.profileId === priority) - Number(a.profileId === priority) ||
        b.count - a.count ||
        a.name.localeCompare(b.name),
    );
  const centroids = new Map(plotted.map((group) => [group.profileId, group]));
  const placed = order.reduce<{ labels: PlacedLabel[]; hidden: string[] }>(
    (state, group) => {
      const target = centroids.get(group.profileId);
      if (!target) return state;
      const lines = wrapLabel(group.name, maxWidth, measure);
      const countText = `${group.count.toLocaleString()} ${group.count === 1 ? "run" : "runs"}`;
      const box = labelSize(lines, countText, measure);
      const spread = Math.max(...target.points.map((point) => Math.hypot(point.x - target.centroid.x, point.y - target.centroid.y)));
      const options = RING_STEPS.flatMap((step, ring) =>
        DIRECTIONS.map((direction, turn) => ({
          box: candidate(target.centroid, direction, Math.min(spread, 60) + step, box),
          rank: ring * DIRECTIONS.length + turn,
        })),
      );
      const best = options
        .filter((option) => inside(option.box, size) && !state.labels.some((label) => overlaps(label.box, option.box)))
        .map((option) => ({ ...option, covered: positions.filter((point) => covers(option.box, point)).length }))
        .sort((a, b) => a.covered - b.covered || a.rank - b.rank)[0];
      if (!best) return { ...state, hidden: [...state.hidden, group.profileId] };
      const edge = nearestEdge(best.box, target.centroid);
      const leader = Math.hypot(edge.x - target.centroid.x, edge.y - target.centroid.y) > Math.min(spread, 60) + LEADER_MIN ? edge : null;
      return {
        ...state,
        labels: [
          ...state.labels,
          { profileId: group.profileId, lines, countText, box: best.box, anchor: target.centroid, leader },
        ],
      };
    },
    { labels: [], hidden: [] },
  );
  return { groups: plotted, ...placed };
}
