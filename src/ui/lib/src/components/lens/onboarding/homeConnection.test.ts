import { describe, expect, it } from "vitest";
import {
  connectionTraceSql,
  emptyConnection,
  restoreConnection,
} from "./homeConnection";

describe("connection trace lookup", () => {
  it("requires a valid key hash and a bounded setup timestamp", () => {
    expect(connectionTraceSql("a".repeat(64), 123)).toContain(
      "fromUnixTimestamp64Milli(123)",
    );
    for (const key of ["", "key' OR 1=1", "a".repeat(63)])
      expect(connectionTraceSql(key, 123)).toBeNull();
    for (const time of [NaN, Infinity, -1, 1.5])
      expect(connectionTraceSql("a".repeat(64), time)).toBeNull();
  });
  it("restores only nonsecret connection progress and recovers from invalid browser storage", () => {
    const progress = { keyId: "key-one", startedAt: 123, existing: true };
    expect(
      restoreConnection(
        JSON.stringify({ ...progress, secret: "must-not-be-restored" }),
      ),
    ).toEqual(progress);
    expect(restoreConnection("invalid")).toEqual(emptyConnection);
    expect(restoreConnection('{"startedAt":"yesterday"}')).toEqual(
      emptyConnection,
    );
  });
});
