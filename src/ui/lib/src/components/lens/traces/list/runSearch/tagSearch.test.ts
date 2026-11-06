import { expect, it } from "vitest";
import { mergeTags, searchTags, withoutSearchTags } from "./tagSearch";

it.each([
  [
    "tag:skill:diagram status:error",
    [{ key: "tag", value: "skill:diagram" }],
    "status:error",
  ],
  [
    'tag:"User-Agent: codex" refund',
    [{ key: "tag", value: "User-Agent: codex" }],
    "refund",
  ],
  ["tag:team*", [{ key: "tag", value: "team*" }], ""],
  ["-tag:secret", [], "-tag:secret"],
])("extracts exact server tag conditions from %s", (query, tags, remaining) => {
  expect(searchTags(query)).toEqual(tags);
  expect(withoutSearchTags(query)).toBe(remaining);
});

it("combines legacy shared-link filters with search tags without duplicates", () => {
  expect(
    mergeTags(
      [
        { key: "tag", value: "diagram" },
        { key: "department", value: "engineering" },
      ],
      "tag:diagram tag:coding",
    ),
  ).toEqual([
    { key: "tag", value: "diagram" },
    { key: "department", value: "engineering" },
    { key: "tag", value: "coding" },
  ]);
});
