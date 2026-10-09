import { readFile, writeFile } from "node:fs/promises";
import { compile } from "json-schema-to-typescript";

const schema = new URL("../schema/lens-traces.json", import.meta.url);
const output = new URL("../src/ui/lib/src/lib/http/traces.d.ts", import.meta.url);
const definitions = JSON.parse(await readFile(schema, "utf8"));
const options = { bannerComment: "", ignoreMinAndMaxItems: true };
const content =
  "/* Generated from schema/lens-traces.json. Run npm run generate:trace-contract. */\n" +
  (await compile(definitions.TraceConversationPage, "TraceConversationPage", options)) +
  (await compile(definitions.TraceConversationRequest, "TraceConversationRequest", options));

if (process.argv.includes("--check")) {
  if ((await readFile(output, "utf8")) !== content)
    throw new Error("Trace types drift from the Rust schema; run npm run generate:trace-contract.");
} else {
  await writeFile(output, content);
}
