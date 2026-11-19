import { describe, expectTypeOf, test } from "vitest";
import type { z } from "zod";
import type * as Wire from "../../../lib/http/worker";
import type { detailSchema, matrixSchema, overviewSchema } from "./schemas";

/** Every field name, at every depth, with optionality and value types erased. */
type Shape<T> = T extends readonly (infer Item)[]
  ? Shape<Item>[]
  : T extends object
    ? { [Key in keyof T]-?: Shape<NonNullable<T[Key]>> }
    : unknown;

describe("profile response schemas follow the generated Rust contract", () => {
  test("overview", () => {
    expectTypeOf<z.infer<typeof overviewSchema>>().toExtend<Wire.ProfilesOverview>();
    expectTypeOf<Shape<z.infer<typeof overviewSchema>>>().toEqualTypeOf<Shape<Wire.ProfilesOverview>>();
  });

  test("matrix", () => {
    expectTypeOf<z.infer<typeof matrixSchema>>().toExtend<Wire.ProfilesMatrix>();
    expectTypeOf<Shape<z.infer<typeof matrixSchema>>>().toEqualTypeOf<Shape<Wire.ProfilesMatrix>>();
  });

  test("detail", () => {
    expectTypeOf<z.infer<typeof detailSchema>>().toExtend<Wire.ProfileDetail>();
    expectTypeOf<Shape<z.infer<typeof detailSchema>>>().toEqualTypeOf<Shape<Wire.ProfileDetail>>();
  });
});
