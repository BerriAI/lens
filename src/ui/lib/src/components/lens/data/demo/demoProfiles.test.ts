import { describe, expect, it } from "vitest";
import { createLensDemo } from "./createLensDemo";

const NOW = Date.UTC(2026, 10, 13, 12);
const everything = { startMs: 0, endMs: NOW + 1 };
const profiles = () => createLensDemo(NOW).profiles;

describe("demo profiles", () => {
  it("builds a matrix whose rows are the overview's profiles and whose cells price only priced runs", async () => {
    const api = profiles();
    const overview = await api.overview(everything, "support_agent");
    const matrix = await api.matrix(everything, "support_agent");

    expect(matrix.profiles.map((profile) => profile.profile_id)).toEqual(overview.profiles.map((profile) => profile.profile_id));
    expect(matrix.cells.length).toBeGreaterThan(0);
    expect(matrix.cells.every((cell) => (cell.avg_cost_usd === null) === (cell.cost_observations === 0))).toBe(true);
    expect(new Set(matrix.models.map((model) => model.column_key))).toEqual(new Set(matrix.cells.map((cell) => cell.column_key)));
  });

  it("opens a profile by id or by its older label link", async () => {
    const api = profiles();
    const [first] = (await api.overview(everything, "support_agent")).profiles;

    const byId = await api.detail(everything, "support_agent", first.profile_id);
    const byName = await api.detail(everything, "support_agent", first.name);

    expect(byName).toEqual(byId);
    expect(byId.runs).toHaveLength(first.run_count);
  });

  it("rejects a profile the agent does not have", async () => {
    await expect(profiles().detail(everything, "support_agent", "recorded:Missing")).rejects.toThrow(
      "Profile not found in this agent and time range",
    );
  });
});
