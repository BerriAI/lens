import { Hash } from "lucide-react";
import { describe, expect, it } from "vitest";
import {
  ALL_OPERATORS,
  EQUALITY_ONLY,
  languageOps,
  parseQuery,
  type QueryLanguage,
} from "./language";
import { completingPrefix, suggest } from "./suggestions";

const language: QueryLanguage<"tag" | "name"> = {
  fields: {
    tag: {
      icon: Hash,
      group: "Run attributes",
      suggestValues: true,
      ops: EQUALITY_ONLY,
    },
    name: { icon: Hash, group: "Run attributes", suggestValues: true },
  },
  ops: ALL_OPERATORS,
};

describe("field-specific operators", () => {
  it("preserves literal stars for tags while other fields retain glob operators", () => {
    const clauses = parseQuery(language, "tag:team* -name:worker*");
    expect(clauses[0]).toMatchObject({
      kind: "field",
      field: "tag",
      op: "eq",
      value: "team*",
    });
    expect(clauses[1]).toMatchObject({
      kind: "field",
      field: "name",
      op: "nglob",
      value: "worker*",
    });
    expect(languageOps(language, "tag")).toEqual(["eq"]);
    expect(languageOps(language, "name")).toEqual([
      "eq",
      "neq",
      "glob",
      "nglob",
    ]);
  });

  it("does not turn an unsupported negated tag into a positive tag condition", () => {
    expect(parseQuery(language, "-tag:diagram")[0]).toMatchObject({
      kind: "text",
      value: "-tag:diagram",
    });
    const menu = suggest(language, "-", 1, () => ({
      values: [],
      loading: false,
    }));
    expect(
      menu?.groups.flatMap((group) => group.items).map((item) => item.field),
    ).toEqual(["name"]);
  });

  it("suggests literal-star tag values and hides unsupported operator hints", () => {
    expect(completingPrefix(language, "tag:*", 5)).toBe("*");
    expect(completingPrefix(language, "name:*", 6)).toBe("");
    const menu = suggest(language, "tag:*", 5, () => ({
      values: ["ordinary", "team*special"],
      loading: false,
    }));
    expect(menu?.groups[0].items.map((item) => item.label)).toEqual([
      "team*special",
    ]);
    expect(
      suggest(language, "tag:", 4, () => ({
        values: ["diagram"],
        loading: false,
      }))?.showOperators,
    ).toBe(false);
  });
});

describe("opaque suggested values", () => {
  it.each([
    "skill:diagram",
    "User-Agent: codex",
    'team "platform"',
    String.raw`team\platform`,
    ' team "platform\\ops" ',
  ])("round-trips %j without altering the selected tag", (value) => {
    const item = suggest(language, "tag:", 4, () => ({
      values: [value],
      loading: false,
    }))?.groups[0].items[0];
    expect(item).toBeDefined();
    const query = `tag:${item!.insert} name:worker`;
    expect(parseQuery(language, query)).toMatchObject([
      { kind: "field", field: "tag", op: "eq", value },
      { kind: "field", field: "name", op: "eq", value: "worker" },
    ]);
  });

  it("keeps an unfinished quoted value available for completion", () => {
    expect(parseQuery(language, 'tag:"User-Agent: codex')[0]).toMatchObject({
      kind: "field",
      field: "tag",
      value: "User-Agent: codex",
    });
  });
});
