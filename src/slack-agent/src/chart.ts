import { fileURLToPath } from "node:url";
import { access } from "node:fs/promises";
import { Resvg } from "@resvg/resvg-js";
import type { WebClient } from "@slack/web-api";

export interface ChartBucket {
  readonly label: string;
  readonly affected: number;
  readonly total: number;
}
export interface ChartPayload {
  readonly title: string;
  readonly category?: string;
  readonly affected: number;
  readonly total: number;
  readonly unit: string;
  readonly affectedLabel?: string;
  readonly otherLabel?: string;
  readonly buckets?: readonly ChartBucket[];
}
export type ChartSlack = Pick<WebClient, "filesUploadV2">;

const fontFile = fileURLToPath(
  new URL("../assets/LensSans.ttf", import.meta.url),
);
const purple = "#4220ed";
const gray = "#80808b";
const xml = (value: string): string =>
  value.replace(
    /[&<>"']/g,
    (character) =>
      ({
        "&": "&amp;",
        "<": "&lt;",
        ">": "&gt;",
        '"': "&quot;",
        "'": "&apos;",
      })[character]!,
  );
const count = (value: number): string => value.toLocaleString("en-US");

function validateText(value: string, maximum: number): void {
  if (
    typeof value !== "string" ||
    !value.trim() ||
    value.length > maximum ||
    /[\p{Cc}\p{Cf}]/u.test(value)
  ) {
    throw new Error("Chart labels must be bounded, visible text.");
  }
}
function validateCounts(affected: number, total: number): void {
  if (
    !Number.isSafeInteger(affected) ||
    !Number.isSafeInteger(total) ||
    affected < 0 ||
    total < affected ||
    total > 1_000_000_000
  ) {
    throw new Error(
      "Chart counts must be integers with 0 <= affected <= total.",
    );
  }
}
function validate(payload: ChartPayload): void {
  validateText(payload.title, 160);
  validateText(payload.unit, 48);
  if (payload.category !== undefined) validateText(payload.category, 32);
  if (payload.affectedLabel !== undefined)
    validateText(payload.affectedLabel, 32);
  if (payload.otherLabel !== undefined) validateText(payload.otherLabel, 32);
  validateCounts(payload.affected, payload.total);
  if (payload.buckets !== undefined) {
    if (payload.buckets.length < 1 || payload.buckets.length > 6) {
      throw new Error("A chart must have between one and six buckets.");
    }
    for (const bucket of payload.buckets) {
      validateText(bucket.label, 32);
      validateCounts(bucket.affected, bucket.total);
    }
    if (
      payload.buckets.reduce((sum, bucket) => sum + bucket.affected, 0) !==
        payload.affected ||
      payload.buckets.reduce((sum, bucket) => sum + bucket.total, 0) !==
        payload.total
    ) {
      throw new Error("Chart buckets must partition the headline counts.");
    }
  }
}
function wrap(value: string, width: number): string[] {
  const lines: string[] = [];
  let line = "";
  for (const word of value.trim().split(/\s+/)) {
    if (line && line.length + word.length + 1 > width) {
      lines.push(line);
      line = "";
    }
    for (let index = 0; index < word.length; index += width) {
      const part = word.slice(index, index + width);
      if (index > 0) {
        lines.push(line);
        line = "";
      }
      line += `${line ? " " : ""}${part}`;
    }
  }
  if (line) lines.push(line);
  return lines;
}
function percentage(affected: number, total: number): string {
  if (total === 0) return "N/A";
  const rate = (100 * affected) / total;
  if (rate > 0 && rate < 0.1) return "<0.1%";
  if (rate < 100 && rate > 99.9) return ">99.9%";
  return `${Math.round(rate * 10) / 10}%`;
}
function description(payload: ChartPayload): string {
  const affected = (payload.affectedLabel ?? "Affected").toLowerCase();
  const other = (payload.otherLabel ?? "Unaffected").toLowerCase();
  const summary = `${payload.title}: ${count(payload.affected)} of ${count(payload.total)} ${payload.unit} ${affected} (${percentage(payload.affected, payload.total)}).`;
  const buckets = payload.buckets?.map(
    (bucket) =>
      `${bucket.label}: ${count(bucket.affected)} ${affected}, ${count(bucket.total - bucket.affected)} ${other}.`,
  );
  return `${summary}${buckets ? ` ${buckets.join(" ")}` : ""} Observed sample; not a projected rate.`;
}

export function renderChartSvg(payload: ChartPayload): string {
  validate(payload);
  const headline = wrap(
    `${payload.category ? `${payload.category} / ` : ""}${payload.title}`,
    82,
  );
  const offset = (headline.length - 1) * 24;
  const height = 700 + offset;
  const buckets = payload.buckets ?? [
    {
      label: "Observed sample",
      affected: payload.affected,
      total: payload.total,
    },
  ];
  const maximum = Math.max(1, ...buckets.map((bucket) => bucket.total));
  const ticks =
    maximum <= 4
      ? Array.from({ length: maximum + 1 }, (_, index) => index)
      : [0, Math.ceil(maximum / 2), maximum];
  const chartLeft = 138;
  const chartWidth = 810;
  const chartBottom = 528;
  const chartHeight = 252;
  const cellWidth = chartWidth / buckets.length;
  const barWidth = Math.min(330, cellWidth * 0.68);
  const subtitle =
    payload.total === 0
      ? "No observations available"
      : `${count(payload.affected)} of ${count(payload.total)} ${payload.unit} ${payload.affectedLabel ? `(${payload.affectedLabel.toLowerCase()})` : "affected"}`;
  const text = (
    x: number,
    y: number,
    value: string,
    size: number,
    color = "#787885",
    anchor = "start",
  ) =>
    `<text x="${x}" y="${y}" font-size="${size}" fill="${color}" text-anchor="${anchor}">${xml(value)}</text>`;
  const bars = buckets
    .map((bucket, index) => {
      const x = chartLeft + cellWidth * index + (cellWidth - barWidth) / 2;
      const affectedHeight = (chartHeight * bucket.affected) / maximum;
      const unaffectedHeight =
        (chartHeight * (bucket.total - bucket.affected)) / maximum;
      return `<rect x="${x}" y="${chartBottom - affectedHeight}" width="${barWidth}" height="${affectedHeight}" fill="${purple}"/>
      <rect x="${x}" y="${chartBottom - affectedHeight - unaffectedHeight}" width="${barWidth}" height="${unaffectedHeight}" fill="${gray}"/>
      ${wrap(bucket.label, buckets.length > 3 ? 14 : 24)
        .map((line, lineIndex) =>
          text(
            x + barWidth / 2,
            chartBottom + 28 + lineIndex * 20,
            line,
            17,
            "#787885",
            "middle",
          ),
        )
        .join("")}`;
    })
    .join("");
  return `<svg xmlns="http://www.w3.org/2000/svg" width="1040" height="${height}" viewBox="0 0 1040 ${height}">
    <title>${xml(description(payload))}</title>
    <rect width="1040" height="${height}" fill="#ffffff"/>
    <g font-family="ABeeZee">
      ${headline.map((line, index) => text(48, 34 + index * 24, line.toUpperCase(), 18)).join("")}
      <g transform="translate(0 ${offset})">
        <rect x="24" y="60" width="992" height="616" rx="20" fill="#fafafa" stroke="#eaeaf0"/>
        ${text(64, 111, "Frequency", 26, "#19191d")}
        ${text(64, 164, percentage(payload.affected, payload.total), 48, "#19191d")}
        ${wrap(subtitle, 49)
          .map((line, index) => text(270, 148 + index * 29, line, 23))
          .join("")}
        <text x="60" y="404" font-size="16" fill="#787885" text-anchor="middle" transform="rotate(-90 60 404)">${xml(payload.unit)}</text>
        ${ticks.map((tick) => text(chartLeft - 18, chartBottom - (chartHeight * tick) / maximum + 6, count(tick), 17, "#787885", "end")).join("")}
        ${bars}
        <line x1="${chartLeft}" y1="${chartBottom}" x2="${chartLeft + chartWidth}" y2="${chartBottom}" stroke="#d6d6dd"/>
        <circle cx="136" cy="615" r="7" fill="${purple}"/>
        ${text(154, 621, payload.affectedLabel ?? "Affected", 18)}
        <circle cx="520" cy="615" r="7" fill="${gray}"/>
        ${text(538, 621, payload.otherLabel ?? "Unaffected", 18)}
        ${text(64, 658, "Observed sample; not a projected rate.", 16)}
      </g>
    </g>
  </svg>`;
}

export async function renderChartPng(payload: ChartPayload): Promise<Buffer> {
  const svg = renderChartSvg(payload);
  await access(fontFile);
  return new Resvg(svg, {
    font: {
      loadSystemFonts: false,
      fontFiles: [fontFile],
      defaultFontFamily: "ABeeZee",
    },
  })
    .render()
    .asPng();
}

export async function prepareChart(slack: ChartSlack, payload: ChartPayload) {
  try {
    const file = await renderChartPng(payload);
    const altText = description(payload);
    const result = await slack.filesUploadV2({
      file,
      filename: "lens-frequency.png",
      title: payload.title,
      alt_text: altText,
    });
    const completion = result.files[0];
    const uploaded = completion?.files?.[0];
    if (
      !result.ok ||
      result.files.length !== 1 ||
      !completion?.ok ||
      completion.files?.length !== 1 ||
      !uploaded?.id ||
      !/^F[A-Z0-9]+$/.test(uploaded.id) ||
      uploaded.is_public === true ||
      uploaded.public_url_shared === true
    ) {
      throw new Error("Invalid private upload receipt.");
    }
    return {
      fileId: uploaded.id,
      block: {
        type: "image" as const,
        slack_file: { id: uploaded.id },
        alt_text: altText,
      },
    };
  } catch {
    throw new Error(
      "The required chart could not be prepared privately in Slack.",
    );
  }
}
