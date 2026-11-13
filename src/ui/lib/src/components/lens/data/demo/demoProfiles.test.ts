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
    expect(byId.definition).toBeNull();
  });

  it("rejects a profile the agent does not have", async () => {
    await expect(profiles().detail(everything, "support_agent", "task:missing")).rejects.toThrow(
      "Profile not found in this agent and time range",
    );
  });

  it("keeps task discovery off and read-only without contacting a model", async () => {
    const api = profiles();
    const settings = await api.settings("support_agent");
    const window = everything;

    expect(settings.settings.enabled).toBe(false);
    expect(settings.can_write).toBe(false);
    expect(await api.taxonomy("support_agent")).toEqual({ taxonomy: null, active: null, discovery_available: false, can_write: false });
    await expect(api.preview({ agent: "support_agent", window, maxRuns: 20 })).rejects.toThrow("The demo is read-only");
    await expect(api.discover({ agent: "support_agent", window, maxRuns: 20 })).rejects.toThrow("The demo is read-only");
    await expect(
      api.saveSettings({ agent: "support_agent", revision: 0, enabled: true, models: settings.defaults }),
    ).rejects.toThrow("The demo is read-only");
    await expect(api.activate("support_agent", 1, false)).rejects.toThrow("The demo is read-only");
  });
});
