import { expect, it } from "vitest";
import { paretoFrontier } from "./model";

it("keeps cheaper and higher-quality tradeoffs and excludes dominated models", () => {
  const cheap = { cost: 1, quality: 7 };
  const balanced = { cost: 2, quality: 8 };
  const best = { cost: 3, quality: 9 };
  expect(paretoFrontier([best, { cost: 4, quality: 8.5 }, cheap, balanced])).toEqual([cheap, balanced, best]);
});

it("handles equal costs, equal scores, and identical results without dropping valid ties", () => {
  const first = { id: "first", cost: 1, quality: 8 };
  const tied = { id: "tied", cost: 1, quality: 8 };
  expect(paretoFrontier([first, { cost: 1, quality: 7 }, { cost: 2, quality: 8 }, tied])).toEqual([first, tied]);
  expect(paretoFrontier([])).toEqual([]);
  expect(paretoFrontier([first])).toEqual([first]);
});
