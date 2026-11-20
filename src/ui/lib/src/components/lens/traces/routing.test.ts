import { expect, it } from "vitest";
import { traceShareUrl } from "./routing";

it("retains the whole workspace hierarchy when sharing a trace", () => {
  const url = new URL(
    traceShareUrl(
      { traceId: "trace-a", traceRef: "ref-a" },
      {
        origin: "https://gateway.test",
        pathname: "/ui/lens",
        search:
          "?organization_id=org-a&team_id=team-a&project_id=project-a&api_key_hash=hash-a&status=error&agent=old-agent",
      },
    ),
  );
  expect(Object.fromEntries(url.searchParams)).toEqual({
    organization_id: "org-a",
    team_id: "team-a",
    project_id: "project-a",
    api_key_hash: "hash-a",
    trace: "trace-a",
    trace_ref: "ref-a",
  });
});
