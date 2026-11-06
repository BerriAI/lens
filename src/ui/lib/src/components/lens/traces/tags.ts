import { z } from "zod";
import { parseAsJson } from "nuqs";
import type { TraceTags } from "../../../lib/http/traces";

const traceTagSchema = z.object({
  key: z.string().min(1).max(256).refine((value) => value.trim().length > 0),
  value: z.string().min(1).max(256).refine((value) => value.trim().length > 0),
});
export const traceTagsSchema = z.array(traceTagSchema).max(8);
export type TraceTag = TraceTags["manual"][number];
export const traceTagsParser = parseAsJson(traceTagsSchema).withDefault([]);
export const traceTagSetSchema = z.object({
  recorded: z.array(traceTagSchema).max(64),
  manual: traceTagsSchema,
  editable: z.boolean(),
}) satisfies z.ZodType<TraceTags>;
