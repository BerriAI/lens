import { beforeEach, describe, expect, it, vi } from "vitest";
import { apiClient } from "../../../../lib/http/requests";
import { createTracingKey } from "./tracingKeys";

vi.mock("../../../../lib/http/requests", () => ({
  apiClient: { post: vi.fn() },
}));

beforeEach(() => {
  vi.mocked(apiClient.post).mockResolvedValue({ key: "new-secret" });
});

describe("createTracingKey", () => {
  it.each([
    ["user-b", { name: "Agent", user_id: "user-b" }],
    [" user-b \n", { name: "Agent", user_id: "user-b" }],
    [undefined, { name: "Agent" }],
    ["", { name: "Agent" }],
    [" \t\n", { name: "Agent" }],
  ])("sends only a non-empty trimmed user id (%j)", async (userId, body) => {
    await createTracingKey("admin-token", "Agent", userId);
    expect(apiClient.post).toHaveBeenLastCalledWith("/lens/tracing/keys", {
      accessToken: "admin-token",
      body,
    });
  });

  it("rejects a response without a secret", async () => {
    vi.mocked(apiClient.post).mockResolvedValue({});
    await expect(
      createTracingKey("admin-token", "Agent", "user-b"),
    ).rejects.toThrow("Lens did not return the new key");
  });

  it("preserves request failures", async () => {
    vi.mocked(apiClient.post).mockRejectedValue(new Error("Not authorized"));
    await expect(
      createTracingKey("admin-token", "Agent", "user-b"),
    ).rejects.toThrow("Not authorized");
  });
});
