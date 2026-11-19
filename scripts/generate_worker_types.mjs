import { readFile, writeFile } from "node:fs/promises";
import { compile } from "json-schema-to-typescript";

const schema = new URL("../schema/lens-worker.v7.json", import.meta.url);
const output = new URL("../src/ui/lib/src/lib/http/worker.d.ts", import.meta.url);
const apiOutput = new URL("../src/ui/lib/src/lib/http/schema.d.ts", import.meta.url);
const content = await compile(JSON.parse(await readFile(schema, "utf8")), "LensWorker", {
  bannerComment: "/* Generated from schema/lens-worker.v7.json. Run npm run generate:worker-contract. */",
  unreachableDefinitions: true,
  ignoreMinAndMaxItems: true,
});

const api = await readFile(apiOutput, "utf8");
const apiContent = ["IngestionKey", "IngestionKeyCreated", "IngestionKeyRequest", "IngestionTenant"].reduce(
  (source, name) => source.replace(
    new RegExp(`        ${name}: (?:\\{[\\s\\S]*?^        \\}|import\\("\\./worker"\\)\\.${name});`, "m"),
    `        ${name}: import("./worker").${name};`,
  ),
  api,
);

if (process.argv.includes("--check")) {
  if ((await readFile(output, "utf8")) !== content) {
    throw new Error("Worker types drift from the Rust schema; run npm run generate:worker-contract.");
  }
  if (api !== apiContent) {
    throw new Error("Ingestion API types drift from the Rust schema; run npm run generate:worker-contract.");
  }
} else {
  await writeFile(output, content);
  await writeFile(apiOutput, apiContent);
}
