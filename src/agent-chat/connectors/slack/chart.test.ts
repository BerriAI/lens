import assert from "node:assert/strict";
import { test } from "node:test";
import { Resvg } from "@resvg/resvg-js";
import {
  prepareChart,
  renderChartPng,
  renderChartSvg,
  type ChartPayload,
  type ChartSlack,
} from "./chart.js";

const payload: ChartPayload = {
  title: "Repository discovery",
  category: "Reliability",
  affected: 4,
  total: 5,
  unit: "invoking user turns",
  buckets: [
    { label: "3 PM", affected: 2, total: 2 },
    { label: "4 PM", affected: 2, total: 3 },
  ],
};
const destination = { channel: "C123", thread: "123.4" };

test("frequency chart renders real PNG pixels with proportional affected and unaffected bars", async () => {
  const png = await renderChartPng(payload);
  assert.deepEqual(
    png.subarray(0, 8),
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
  );
  assert.equal(png.readUInt32BE(16), 1040);
  assert.equal(png.readUInt32BE(20), 700);
  assert(png.length > 10_000);
  const svg = renderChartSvg(payload);
  assert(svg.includes("80%"));
  assert(svg.includes("4 of 5 invoking user turns affected"));
  const { pixels, width } = new Resvg(svg).render();
  const pixel = (x: number, y: number) => [
    ...pixels.subarray((y * width + x) * 4, (y * width + x) * 4 + 4),
  ];
  assert.deepEqual(pixel(300, 450), [66, 32, 237, 255]);
  assert.deepEqual(pixel(750, 450), [66, 32, 237, 255]);
  assert.deepEqual(pixel(750, 300), [128, 128, 139, 255]);
  assert.deepEqual(pixel(300, 300), [250, 250, 250, 255]);
});

test("chart escapes labels and represents an empty denominator without inventing a percentage", async () => {
  const svg = renderChartSvg({
    title: '<script>& "hello"',
    affected: 0,
    total: 0,
    unit: "turns",
  });
  assert(!svg.includes("<script>"));
  assert(svg.includes("&lt;script&gt;&amp; &quot;hello&quot;"));
  assert(svg.includes("No observations available"));
  assert(svg.includes("N/A"));
  assert(
    !svg.includes("NaN") && !svg.includes("Infinity") && !svg.includes("0%"),
  );
  assert(
    (
      await renderChartPng({
        title: "Empty sample",
        affected: 0,
        total: 0,
        unit: "turns",
      })
    ).length > 1000,
  );
});

test("feature request chart distinguishes support from other reviewed requests", async () => {
  const feature: ChartPayload = {
    title: "Message feedback",
    affected: 1,
    total: 20,
    unit: "readable current requests",
    affectedLabel: "Supporting requests",
    otherLabel: "Other reviewed requests",
  };
  const svg = renderChartSvg(feature);
  assert(svg.includes("5%"));
  assert(svg.includes("Supporting requests"));
  assert(svg.includes("Other reviewed requests"));
  assert(!svg.toLowerCase().includes("unaffected"));
  const prepared = await prepareChart(
    {
      filesUploadV2: async () => ({
        ok: true,
        files: [{ ok: true, files: [{ id: "F0123ABC" }] }],
      }),
    },
    feature,
    destination,
  );
  assert(prepared.block.alt_text.includes("supporting requests"));
  assert(!prepared.block.alt_text.includes("unaffected"));
});

test("inconsistent, unbounded and misleading counts fail before upload", async () => {
  let uploads = 0;
  const slack: ChartSlack = {
    filesUploadV2: async () => {
      uploads += 1;
      return { ok: true, files: [] };
    },
  };
  const invalid: readonly ChartPayload[] = [
    { ...payload, affected: -1 },
    { ...payload, affected: 6 },
    { ...payload, affected: 0.5 },
    { ...payload, total: Number.NaN },
    { ...payload, total: Number.POSITIVE_INFINITY },
    { ...payload, total: 1_000_000_001 },
    { ...payload, affected: 3 },
    { ...payload, buckets: [] },
    { ...payload, title: "" },
    { ...payload, title: "hidden\u202etext" },
    { ...payload, unit: "a".repeat(49) },
  ];
  for (const item of invalid)
    await assert.rejects(
      prepareChart(slack, item, destination),
      /required chart could not/,
    );
  assert.equal(uploads, 0);
});

test("chart upload grants the configured channel and thread access without an internet-public URL", async () => {
  const slack: ChartSlack = {
    filesUploadV2: async (options) => {
      assert(Buffer.isBuffer("file" in options ? options.file : undefined));
      assert.equal(options.filename, "lens-frequency.png");
      assert.equal(
        options.alt_text?.includes("3 PM: 2 affected, 0 unaffected."),
        true,
      );
      assert.equal(options.channel_id, destination.channel);
      assert.equal(options.thread_ts, destination.thread);
      assert.equal("channels" in options, false);
      assert.equal("initial_comment" in options, false);
      return {
        ok: true,
        files: [
          {
            ok: true,
            files: [
              { id: "F0123ABC", is_public: true, public_url_shared: false },
            ],
          },
        ],
      };
    },
  };
  const prepared = await prepareChart(slack, payload, destination);
  assert.equal(prepared.fileId, "F0123ABC");
  assert.deepEqual(prepared.block.slack_file, { id: "F0123ABC" });
  assert.equal(prepared.block.type, "image");
  assert.equal("image_url" in prepared.block, false);
  assert(
    prepared.block.alt_text.includes(
      "4 of 5 invoking user turns affected (80%)",
    ),
  );
});

test("failed or public upload receipts fail closed without disclosing vendor errors", async () => {
  const responses: Awaited<ReturnType<ChartSlack["filesUploadV2"]>>[] = [
    { ok: false, files: [] },
    { ok: true, files: [] },
    { ok: true, files: [{ ok: false, files: [{ id: "F0123ABC" }] }] },
    { ok: true, files: [{ ok: true, files: [{}] }] },
    {
      ok: true,
      files: [
        { ok: true, files: [{ id: "F0123ABC", public_url_shared: true }] },
      ],
    },
    { ok: true, files: [{ ok: true, files: [{ id: "https://example.com" }] }] },
  ];
  for (const response of responses) {
    await assert.rejects(
      prepareChart(
        { filesUploadV2: async () => response },
        payload,
        destination,
      ),
      /required chart could not/,
    );
  }
  await assert.rejects(
    prepareChart(
      {
        filesUploadV2: async () => {
          throw new Error("secret vendor response");
        },
      },
      payload,
      destination,
    ),
    (error: Error) =>
      error.message ===
      "The required chart could not be prepared privately in Slack.",
  );
});
