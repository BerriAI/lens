import { Tags } from "lucide-react";
import { EQUALITY_ONLY, parseQuery } from "../../../../shared/search/language";
import type { TraceTag } from "../../tags";

export const TAG_FIELD = {
  group: "Run attributes",
  icon: Tags,
  suggestValues: true,
  ops: EQUALITY_ONLY,
} as const;
const TAG_LANGUAGE = { fields: { tag: TAG_FIELD }, ops: EQUALITY_ONLY };

export function searchTags(query: string): TraceTag[] {
  return parseQuery(TAG_LANGUAGE, query).flatMap((clause) =>
    clause.kind === "field" && clause.value
      ? [{ key: "tag", value: clause.value }]
      : [],
  );
}

export function withoutSearchTags(query: string): string {
  return parseQuery(TAG_LANGUAGE, query)
    .filter((clause) => clause.kind !== "field")
    .map((clause) => query.slice(clause.from, clause.to))
    .join(" ");
}

export function mergeTags(
  tags: readonly TraceTag[],
  query: string,
): TraceTag[] {
  return [...tags, ...searchTags(query)].filter(
    (tag, index, all) =>
      all.findIndex(
        (other) => other.key === tag.key && other.value === tag.value,
      ) === index,
  );
}
