import assert from "node:assert/strict";
import { test } from "node:test";
import { setImmediate } from "node:timers/promises";
import OpenAI from "openai";
import {
  bootstrapKnownFindings,
  cycle,
  initialState,
  type State,
  stateSchema,
  startInvestigator,
} from "./investigator.js";
import { digest, type Snapshot, type StateStore } from "./state.js";
import { Evidence, type Sample } from "./evidence.js";
import { verify, type Candidate, type VerifiedCandidate } from "./findings.js";
import { LensClient, LensReadError } from "./lens.js";
import { Repository } from "./repository.js";
import type { AgentConfig as Config } from "./config.js";

const trace = (id: string) => ({
  trace_id: id,
  trace_ref: "ref",
  agent_names: ["selected"],
  service: "selected",
  start_time: "2026-01-01",
  duration_ms: 100,
  status: "error",
  llm_calls: 2,
  tool_calls: 1,
  error_count: 1,
  input_tokens: 20,
  output_tokens: 10,
  url: "https://lens.example.com",
});
const sample: Sample = {
  agent: "selected",
  window: "all retained history",
  scanned_rows: 2,
  matched_rows: 2,
  incomplete: false,
  sampled_for_detail: false,
  population: {
    inspected_traces: 2,
    root_error_traces: 2,
    span_error_traces: 2,
    terminal_traces: 2,
    p95_duration_ms: null,
    above_p95: null,
  },
  warning: "bounded",
  traces: [trace("a"), trace("b")],
};
const candidate: Candidate = {
  kind: "issue",
  category: "Reliability",
  confidence: 0.9,
  impact: 6,
  frequency_metric: "unknown",
  issue_key: "tool-argument-mismatch",
  title: "Tool argument mismatch",
  observation: "Two sampled runs show argument failures",
  code_hypothesis: "The handler expects a missing argument",
  experiment:
    "Freeze both cases and test baseline versus candidate with outcome assertions and held-out cases",
  outcome: "Users could finish repository discovery without retrying",
  limitation: "The fix is unvalidated and the sample is small",
  evidence: [
    { trace_id: "a", span_id: "span-a", quote: "missing argument" },
    { trace_id: "b", span_id: "span-b", quote: "missing argument" },
  ],
  code: [{ path: "src/tool.ts", quote: "function tool(input)" }],
};
const verified: VerifiedCandidate = {
  candidate,
  fingerprint: "fingerprint",
  evidenceHash: "evidence",
  evidenceLinks: ["https://lens.example.com"],
  codeLinks: ["https://github.com/example/repo/blob/sha/src/tool.ts"],
};
const native = {
  lensId: "lens-selected",
  findingId: "agent-" + "a".repeat(64),
  url:
    "https://lens.example.com/?tab=findings&issue=lens-selected:agent-" +
    "a".repeat(64),
};
const persist = async () => native;
class MemoryState implements StateStore<State> {
  snapshot: Snapshot<State> = {
    key: "test",
    revision: 0,
    digest: "",
    value: initialState,
  };
  async read() {
    return this.snapshot;
  }
  async commit(previous: Snapshot<State>, value: State) {
    if (previous.revision !== this.snapshot.revision)
      throw new Error("conflict");
    this.snapshot = {
      key: previous.key,
      revision: previous.revision + 1,
      digest: digest(JSON.stringify(value)),
      value,
    };
    return this.snapshot;
  }
}

test(
  "scheduled startup preserves a verified finding with unknown frequency through native persistence and delivery",
  { timeout: 3000 },
  async (context) => {
    context.mock.timers.enable({
      apis: ["Date", "setTimeout"],
      now: 1_800_000_000_000,
    });
    const store = new MemoryState();
    let analyzeStarted!: (signal: AbortSignal) => void;
    const analyzing = new Promise<AbortSignal>((resolve) => {
      analyzeStarted = resolve;
    });
    let finish!: () => void;
    const finished = new Promise<void>((resolve) => {
      finish = resolve;
    });
    let deliver!: () => void;
    const delivered = new Promise<void>((resolve) => {
      deliver = resolve;
    });
    const bodies: Record<string, unknown>[] = [];
    const config = {
      agent: "selected",
      apiUrl: "https://lens.example.com",
      publicUrl: "https://lens.example.com",
      lensKey: "fixture",
    } as Config;
    const client = new LensClient(config, async (_url, init) => {
      if (init?.method === "POST") {
        bodies.push(JSON.parse(String(init.body)));
        return new Response(
          JSON.stringify({
            lens_id: native.lensId,
            finding_id: native.findingId,
          }),
        );
      }
      return new Response(
        JSON.stringify({
          lenses: [
            {
              id: native.lensId,
              settings: { agent_name: "selected" },
              findings: [],
            },
          ],
        }),
      );
    });
    const stop = startInvestigator(
      config,
      {
        repository: "example/repo",
        clientId: "fixture",
        key: "fixture",
        dailyLimit: 1,
        model: "fixture",
        scope: ["selected"],
        stateConnection: "http://localhost",
        stateDatabase: "fixture",
      },
      async (finding) => {
        assert.equal(finding.frequency, undefined);
        assert.equal(finding.native.findingId, native.findingId);
        deliver();
      },
      {
        store,
        client: {
          recent: async () => sample,
          get: client.get.bind(client),
          link: client.link.bind(client),
          persistFinding: client.persistFinding.bind(client),
        },
        repository: async () =>
          new Repository("example/repo", "fixture", "fixture"),
        investigate: async (
          _config,
          _model,
          _evidence,
          _repo,
          _previous,
          signal,
        ) => {
          analyzeStarted(signal);
          await finished;
          return [{ ...verified, fingerprint: digest("verified finding") }];
        },
      },
    );
    context.after(stop);
    const signal = await analyzing;
    for (let minute = 0; minute < 15; minute++) {
      context.mock.timers.tick(60_000);
      await setImmediate();
      assert.equal(signal.aborted, false);
      assert(store.snapshot.value.lease_until > Date.now());
    }
    await cycle(
      {
        store,
        model: "fixture",
        dailyLimit: 1,
        sample: async () =>
          assert.fail("An active investigation must keep ownership"),
        analyze: async () => assert.fail("Duplicate analysis"),
        persist,
        post: async () => assert.fail("Duplicate post"),
      },
      new AbortController().signal,
    );
    finish();
    await delivered;
    assert.equal(bodies.length, 1);
    assert.match(String(bodies[0]?.description), /Frequency: unknown/);
    assert.doesNotMatch(String(bodies[0]?.description), /0\/0|undefined|NaN/);
    assert.equal(store.snapshot.value.sent.length, 1);
    assert.equal(store.snapshot.value.runs, 1);
  },
);

test("finding headlines are independent of measured cohorts while invalid metric selections fail closed", async () => {
  const traces = [trace("a"), trace("b"), { ...trace("c"), status: "ok" }];
  const evidence = new Evidence(
    { ...sample, traces },
    {
      link: (query) =>
        `https://lens.example.com/?${new URLSearchParams(query)}`,
      get: async (path) => {
        const id = path.split("/")[3]!.split("?")[0]!;
        return path.includes("/spans/")
          ? {
              span_id: `span-${id}`,
              input: JSON.stringify([
                { role: "user", content: "Inspect repository" },
              ]),
              output: "tool returned missing argument",
              attributes: {
                "session.id": `incident-${id}`,
                "gen_ai.operation.name": "invoke_agent",
              },
            }
          : {
              summary: { trace_id: id, agent_names: ["selected"] },
              spans: [
                {
                  span_id: `span-${id}`,
                  parent_span_id: null,
                  name: "selected",
                  type: "agent",
                  status: id === "c" ? "ok" : "error",
                  duration_ms: 100,
                  error: null,
                },
              ],
              next_cursor: null,
            };
      },
    },
    "selected",
  );
  await evidence.loadRoots(AbortSignal.timeout(1000));
  const repo = {
    files: new Map([
      [
        "src/tool.ts",
        {
          path: "src/tool.ts",
          content: "function tool(input) {}",
          url: "https://github.com/example/repo/blob/sha/src/tool.ts",
        },
      ],
    ]),
  };
  const measured = verify(
    { ...candidate, frequency_metric: "root_errors" },
    evidence,
    repo,
  );
  assert(measured);
  assert.equal(measured.candidate.title, candidate.title);
  assert.equal(measured.frequency?.count, 2);
  assert.equal(measured.frequency?.total, 3);
  assert.notEqual(measured.frequency?.title, candidate.title);
  assert.deepEqual(
    measured.frequency,
    evidence.report().metrics.find((metric) => metric.id === "root_errors"),
  );
  const unknown = verify(candidate, evidence, repo);
  assert(unknown);
  assert.equal(unknown.frequency, undefined);
  for (const invalid of [
    { ...candidate, frequency_metric: "unavailable_metric" },
    { ...candidate, frequency_metric: "" },
    {
      ...candidate,
      frequency_metric: "root_errors",
      category: "Performance" as const,
    },
    {
      ...candidate,
      frequency_metric: "root_errors",
      evidence: [
        candidate.evidence[0]!,
        { trace_id: "c", span_id: "span-c", quote: "missing argument" },
      ],
    },
    { ...candidate, evidence: [candidate.evidence[0]!] },
  ])
    assert.equal(verify(invalid, evidence, repo), undefined);
});

test("scoped known manual reports initialize only empty state and suppress the same measured finding", async () => {
  const scope = ["workspace", "channel", "selected", "example/repo"];
  const seed = JSON.stringify({
    scope,
    findings: [
      {
        fingerprint: "prior-fingerprint",
        evidence_hash: "prior-evidence",
        issue_key: "prior-key",
        title: candidate.title,
        issue_number: 2026100901,
        provenance: {
          model: "model",
          prompt_revision: "version",
          trace_ids: ["a"],
          repository_sha: "sha",
          at: 1,
        },
      },
    ],
  });
  const store = new MemoryState();
  await assert.rejects(
    bootstrapKnownFindings(store, ["other", ...scope.slice(1)], seed),
    /scope/,
  );
  assert.equal(store.snapshot.revision, 0);
  await bootstrapKnownFindings(store, scope, seed);
  assert.equal(store.snapshot.value.sent[0]?.issue_number, 2026100901);
  let posts = 0;
  await cycle(
    {
      store,
      persist,
      sample: async () => sample,
      analyze: async () => ({ candidates: [verified], sha: "sha" }),
      post: async () => {
        posts++;
      },
      model: "model",
      dailyLimit: 144,
      now: () => 1_800_000_000_000,
    },
    AbortSignal.timeout(1000),
  );
  assert.equal(posts, 0);
  assert.deepEqual(store.snapshot.value.sent[0]?.native_finding, native);
  const preserved = structuredClone(store.snapshot);
  await bootstrapKnownFindings(
    store,
    scope,
    seed.replace(candidate.title, "Changed title"),
  );
  assert.deepEqual(store.snapshot, preserved);
  assert.equal(store.snapshot.value.next_issue_number, 1);
});

test("CAS claims prevent overlapping paid investigations and survive a new worker instance", async () => {
  const store = new MemoryState();
  let analyses = 0;
  let posts = 0;
  const deps = {
    store,
    persist,
    sample: async () => sample,
    analyze: async () => {
      analyses++;
      return { candidates: [verified], sha: "pinned" };
    },
    post: async () => {
      posts++;
    },
    model: "model",
    dailyLimit: 12,
    now: () => 1_800_000_000_000,
  };
  await Promise.allSettled([
    cycle(deps, AbortSignal.timeout(1000)),
    cycle(deps, AbortSignal.timeout(1000)),
  ]);
  await cycle({ ...deps }, AbortSignal.timeout(1000));
  assert.equal(analyses, 1);
  assert.equal(posts, 1);
  assert.equal(store.snapshot.value.runs, 1);
  assert.equal(store.snapshot.value.sent[0]?.issue_number, 1);
  assert.equal(store.snapshot.value.next_issue_number, 2);
  assert.equal(
    store.snapshot.value.sent[0]?.provenance.repository_sha,
    "pinned",
  );
});

test("unchanged traces and repeated candidates stay quiet across later cycles", async () => {
  const store = new MemoryState();
  let time = 1_800_000_000_000;
  let analyses = 0;
  let posts = 0;
  const deps = {
    store,
    persist,
    sample: async () => sample,
    analyze: async () => {
      analyses++;
      return { candidates: [verified], sha: "pinned" };
    },
    post: async () => {
      posts++;
    },
    model: "model",
    dailyLimit: 12,
    now: () => time,
  };
  await cycle(deps, AbortSignal.timeout(1000));
  time += 600_001;
  await cycle(deps, AbortSignal.timeout(1000));
  assert.equal(analyses, 1);
  time += 600_001;
  await cycle(
    { ...deps, sample: async () => ({ ...sample, traces: [trace("new")] }) },
    AbortSignal.timeout(1000),
  );
  assert.equal(analyses, 2);
  assert.equal(posts, 1);
});

test("daily paid-run limit persists and quiet results do not post", async () => {
  const store = new MemoryState();
  let time = 1_800_000_000_000;
  let analyses = 0;
  let posts = 0;
  const deps = {
    store,
    persist,
    sample: async () => ({ ...sample, traces: [trace(String(time))] }),
    analyze: async () => {
      analyses++;
      return { candidates: [], sha: "pinned" };
    },
    post: async () => {
      posts++;
    },
    model: "model",
    dailyLimit: 1,
    now: () => time,
  };
  await cycle(deps, AbortSignal.timeout(1000));
  time += 600_001;
  await cycle(deps, AbortSignal.timeout(1000));
  assert.equal(analyses, 1);
  assert.equal(posts, 0);
  assert.equal(store.snapshot.value.last_outcome, "run_limit");
  assert.equal(store.snapshot.value.lease_until, 0);
  assert.equal(
    store.snapshot.value.next_at,
    Date.parse(new Date(time).toISOString().slice(0, 10) + "T00:00:00Z") +
      86_400_000,
  );
});

test("the fourth distinct finding is analyzed, persisted and sent instead of silently stopping at three posts", async () => {
  const store = new MemoryState();
  const time = 1_800_000_000_000;
  store.snapshot = {
    ...store.snapshot,
    value: {
      ...initialState,
      day: new Date(time).toISOString().slice(0, 10),
      posts: 3,
    },
  };
  let analyses = 0;
  let imports = 0;
  let posts = 0;
  await cycle(
    {
      store,
      sample: async () => sample,
      analyze: async () => {
        analyses++;
        return { candidates: [verified], sha: "sha" };
      },
      persist: async () => {
        imports++;
        return native;
      },
      post: async () => {
        posts++;
      },
      model: "model",
      dailyLimit: 144,
      now: () => time,
    },
    AbortSignal.timeout(1000),
  );
  assert.equal(analyses, 1);
  assert.equal(imports, 1);
  assert.equal(posts, 1);
  assert.equal(store.snapshot.value.posts, 4);
  assert.equal(store.snapshot.value.lease_until, 0);
  assert.equal(store.snapshot.value.last_outcome, "delivered");
});

test("verified repeated findings refresh native evidence without another Slack notification", async () => {
  const store = new MemoryState();
  store.read = async () => structuredClone(store.snapshot);
  let time = 1_800_000_000_000;
  let posts = 0;
  const fingerprints: string[] = [];
  const firstFingerprint = "a".repeat(64);
  const deps = {
    store,
    sample: async () => ({ ...sample, traces: [trace(String(time))] }),
    analyze: async () => ({
      candidates: [
        {
          ...verified,
          fingerprint: posts ? "b".repeat(64) : firstFingerprint,
          evidenceHash: String(time),
        },
      ],
      sha: "sha",
    }),
    persist: async (item: VerifiedCandidate) => {
      fingerprints.push(item.fingerprint);
      return native;
    },
    post: async () => {
      posts++;
    },
    model: "model",
    dailyLimit: 12,
    now: () => time,
  };
  await cycle(deps, AbortSignal.timeout(1000));
  time += 600_001;
  await cycle(deps, AbortSignal.timeout(1000));
  assert.deepEqual(fingerprints, [firstFingerprint, firstFingerprint]);
  assert.equal(posts, 1);
  assert.equal(store.snapshot.value.posts, 1);
  assert.equal(store.snapshot.value.sent.length, 1);
  assert.equal(store.snapshot.value.sent[0]?.provenance.at, time);
  assert.equal(store.snapshot.value.sent[0]?.evidence_hash, String(time));
  assert.deepEqual(store.snapshot.value.sent[0]?.native_finding, native);
});

test("a failed Slack attempt is persisted before sending and cannot be blindly replayed", async () => {
  const store = new MemoryState();
  let time = 1_800_000_000_000;
  let attempts = 0;
  const deps = {
    store,
    persist,
    sample: async () => ({ ...sample, traces: [trace(String(time))] }),
    analyze: async () => ({ candidates: [verified], sha: "pinned" }),
    post: async () => {
      attempts++;
      throw new Error("lost Slack response");
    },
    model: "model",
    dailyLimit: 12,
    now: () => time,
  };
  await assert.rejects(cycle(deps, AbortSignal.timeout(1000)));
  assert.equal(store.snapshot.value.last_outcome, "delivery_unknown");
  assert.equal(store.snapshot.value.lease_until, 0);
  time += 600_001;
  await cycle(deps, AbortSignal.timeout(1000));
  assert.equal(attempts, 1);
  assert.equal(store.snapshot.value.posts, 1);
});

test("expired ownership cannot post after a long model request", async () => {
  const store = new MemoryState();
  let time = 1_800_000_000_000;
  let posts = 0;
  await assert.rejects(
    cycle(
      {
        store,
        persist,
        sample: async () => sample,
        analyze: async () => {
          time += 300_000;
          return { candidates: [verified], sha: "pinned" };
        },
        post: async () => {
          posts++;
        },
        model: "model",
        dailyLimit: 12,
        now: () => time,
      },
      AbortSignal.timeout(1000),
    ),
    /ownership lost/,
  );
  assert.equal(posts, 0);
});

test("failed analysis leaves evidence eligible for the next cycle and still charges the reserved run", async () => {
  const store = new MemoryState();
  let time = 1_800_000_000_000;
  let attempts = 0;
  const deps = {
    store,
    persist,
    sample: async () => sample,
    analyze: async () => {
      if (++attempts === 1) throw new Error("temporary provider failure");
      return { candidates: [], sha: "pinned" };
    },
    post: async () => {},
    model: "model",
    dailyLimit: 12,
    now: () => time,
  };
  await assert.rejects(cycle(deps, AbortSignal.timeout(1000)));
  assert.equal(store.snapshot.value.signature, "");
  assert.equal(store.snapshot.value.last_outcome, "failed");
  assert.equal(store.snapshot.value.lease_until, 0);
  time += 600_001;
  await cycle(deps, AbortSignal.timeout(1000));
  assert.equal(attempts, 2);
  assert.equal(store.snapshot.value.runs, 2);
  assert.notEqual(store.snapshot.value.signature, "");
});

test("failure status distinguishes response limits and model failures without storing sensitive error content", async () => {
  const cases = [
    { error: new LensReadError("size"), expected: "Lens read exceeds limit" },
    {
      error: new OpenAI.APIConnectionTimeoutError(),
      expected: "Model gateway connection or request timed out",
    },
    {
      error: new OpenAI.APIError(
        403,
        { message: "sensitive provider response" },
        "sensitive model prompt",
        new Headers(),
      ),
      expected: "Model request failed (HTTP 403)",
    },
    {
      error: new Error("sensitive trace and credentials"),
      expected: "Analysis could not finish",
    },
  ];
  for (const { error, expected } of cases) {
    const store = new MemoryState();
    await assert.rejects(
      cycle(
        {
          store,
          persist,
          sample: async () => sample,
          analyze: async () => {
            throw error;
          },
          post: async () => assert.fail("Failed analysis must not post"),
          model: "fixture",
          dailyLimit: 12,
          now: () => 1_800_000_000_000,
        },
        new AbortController().signal,
      ),
    );
    assert(store.snapshot.value.last_error.startsWith(expected));
    assert(!JSON.stringify(store.snapshot.value).includes("sensitive"));
    assert.equal(store.snapshot.value.last_outcome, "failed");
    assert.equal(store.snapshot.value.posts, 0);
    assert.equal(store.snapshot.value.signature, "");
  }
});

test("native import failures do not reserve a Slack send and retry the same verified identity", async () => {
  const store = new MemoryState();
  let time = 1_800_000_000_000;
  let imports = 0;
  let posts = 0;
  const deps = {
    store,
    sample: async () => sample,
    analyze: async () => ({ candidates: [verified], sha: "pinned" }),
    persist: async (item: VerifiedCandidate) => {
      assert.equal(item.fingerprint, verified.fingerprint);
      if (++imports === 1) throw new Error("ambiguous native response");
      return native;
    },
    post: async (item: VerifiedCandidate & { native: typeof native }) => {
      assert.deepEqual(item.native, native);
      posts++;
    },
    model: "model",
    dailyLimit: 12,
    now: () => time,
  };
  await assert.rejects(cycle(deps, AbortSignal.timeout(1000)));
  assert.equal(store.snapshot.value.posts, 0);
  assert.equal(store.snapshot.value.sent.length, 0);
  assert.equal(store.snapshot.value.signature, "");
  assert.equal(store.snapshot.value.runs, 1);
  time += 600_001;
  await cycle(deps, AbortSignal.timeout(1000));
  assert.equal(imports, 2);
  assert.equal(posts, 1);
  assert.deepEqual(store.snapshot.value.sent[0]?.native_finding, native);
});

test("a lost lease after native persistence prevents Slack reservation and delivery", async () => {
  const store = new MemoryState();
  let posts = 0;
  await assert.rejects(
    cycle(
      {
        store,
        sample: async () => sample,
        analyze: async () => ({ candidates: [verified], sha: "pinned" }),
        persist: async () => {
          const previous = await store.read();
          await store.commit(previous, {
            ...previous.value,
            owner: "replacement-worker",
          });
          return native;
        },
        post: async () => {
          posts++;
        },
        model: "model",
        dailyLimit: 12,
        now: () => 1_800_000_000_000,
      },
      AbortSignal.timeout(1000),
    ),
    /ownership lost/,
  );
  assert.equal(posts, 0);
  assert.equal(store.snapshot.value.posts, 0);
  assert.equal(store.snapshot.value.sent.length, 0);
});

test("quotes and code must exist in read evidence; confidence cannot bypass verification", () => {
  const evidence = new Evidence(
    sample,
    new LensClient({} as Config),
    "selected",
  );
  for (const id of ["a", "b"])
    evidence.observations.set(`${id}:span-${id}`, {
      trace_id: id,
      span_id: `span-${id}`,
      input: "same task",
      output: "tool returned missing argument",
      error: "",
      inputHash: digest("same task"),
      root: true,
      incident: `incident-${id}`,
      url: `https://lens.example.com/?trace=${id}`,
    });
  const repo = {
    files: new Map([
      [
        "src/tool.ts",
        {
          path: "src/tool.ts",
          content: "first line\nfunction tool(input) {}",
          url: "https://github.com/example/repo/blob/sha/src/tool.ts",
        },
      ],
    ]),
  };
  assert(verify(candidate, evidence, repo)?.codeLinks[0]?.endsWith("#L2"));
  assert.equal(
    verify({ ...candidate, confidence: 0.84 }, evidence, repo),
    undefined,
  );
  assert.equal(
    verify(
      {
        ...candidate,
        evidence: [{ ...candidate.evidence[0]!, quote: "invented quote" }],
      },
      evidence,
      repo,
    ),
    undefined,
  );
  assert.equal(
    verify(
      {
        ...candidate,
        code: [{ path: "src/never-read.ts", quote: "function tool(input)" }],
      },
      evidence,
      repo,
    ),
    undefined,
  );
  assert.equal(
    verify(
      { ...candidate, observation: "This fixes it 30% faster" },
      evidence,
      repo,
    ),
    undefined,
  );
  assert.equal(
    verify({ ...candidate, kind: "regression" }, evidence, repo)?.candidate
      .kind,
    "issue",
  );
  const single = {
    ...candidate,
    kind: "frustration" as const,
    evidence: [candidate.evidence[0]!],
  };
  assert.equal(verify(single, evidence, repo), undefined);
  const observation = evidence.observations.get("a:span-a")!;
  evidence.observations.set("a:span-a", {
    ...observation,
    input: 'user: "This is still broken and wasting my time"',
    humanInput: ["This is still broken and wasting my time"],
  });
  assert(
    verify(
      {
        ...single,
        evidence: [
          { ...single.evidence[0]!, quote: "still broken and wasting my time" },
        ],
      },
      evidence,
      repo,
    ),
  );
  evidence.observations.set("a:span-a", observation);
  const running = new Evidence(
    {
      ...sample,
      traces: [
        { ...trace("a"), status: "ok", start_time: "2026-01-01" },
        {
          ...trace("b"),
          status: "running",
          start_time: "2026-01-02",
          duration_ms: 250,
        },
      ],
    },
    new LensClient({} as Config),
    "selected",
  );
  for (const [key, value] of evidence.observations)
    running.observations.set(key, value);
  assert.equal(
    verify({ ...candidate, kind: "regression" }, running, repo)?.candidate.kind,
    "issue",
  );
});

test("trace and span reads cannot escape the selected sample or expose credential-shaped text", async () => {
  let reads = 0;
  const client = new LensClient(
    {
      apiUrl: "http://127.0.0.1",
      agent: "selected",
      publicUrl: "https://lens.example.com",
      lensKey: "key",
    } as Config,
    async (url) => {
      reads++;
      return new Response(
        JSON.stringify(
          String(url).includes("/spans/")
            ? { span_id: "s", input: "key=sk-abc123secret", output: "ok" }
            : {
                summary: { trace_id: "a", agent_names: ["selected"] },
                spans: [
                  {
                    span_id: "s",
                    parent_span_id: null,
                    name: "agent",
                    status: "error",
                    duration_ms: 10,
                    error: null,
                    input_preview: "",
                  },
                ],
                next_cursor: null,
              },
        ),
      );
    },
  );
  const evidence = new Evidence(sample, client, "selected");
  await evidence.trace("not-in-sample", AbortSignal.timeout(1000));
  await evidence.span("a", "never-read", AbortSignal.timeout(1000));
  assert.equal(reads, 0);
  await evidence.trace("a", AbortSignal.timeout(1000));
  const result = await evidence.span("a", "s", AbortSignal.timeout(1000));
  assert.equal(reads, 2);
  assert(!result.includes("sk-abc123secret"));
});

test("stored investigator state from before status reporting remains readable", () => {
  const { last_outcome, last_check_at, last_error, ...legacy } = initialState;
  const parsed = stateSchema.parse(legacy);
  assert.equal(parsed.last_outcome, "starting");
  assert.equal(parsed.last_check_at, 0);
  assert.equal(parsed.posts, legacy.posts);
});

test("a daily review sends only its two strongest findings and cannot burst across midnight or restart", async () => {
  const store = new MemoryState();
  let time = Date.parse("2026-01-01T23:55:00Z");
  let analyses = 0;
  const posts: string[] = [];
  const finding = (key: string, impact: number): VerifiedCandidate => ({
    ...verified,
    fingerprint: digest(key),
    evidenceHash: digest(`evidence-${key}`),
    candidate: { ...candidate, issue_key: key, title: key, impact },
  });
  const deps = {
    store,
    persist,
    sample: async () => ({ ...sample, traces: [trace(String(time))] }),
    analyze: async () => {
      analyses++;
      return {
        candidates: [
          finding("minor", 5),
          finding("highest", 8),
          finding("second", 7),
        ],
        sha: "pinned",
      };
    },
    post: async (item: VerifiedCandidate) => {
      posts.push(item.candidate.title);
    },
    model: "model",
    dailyLimit: 144,
    intervalMinutes: 1440,
    maxFindings: 2,
    now: () => time,
  };
  await cycle(deps, AbortSignal.timeout(1000));
  assert.deepEqual(posts, ["highest", "second"]);
  assert.equal(store.snapshot.value.posts, 2);
  assert.equal(store.snapshot.value.sent[1]?.issue_number, 2);
  const next = store.snapshot.value.next_at;
  time += 600_001;
  await cycle({ ...deps }, AbortSignal.timeout(1000));
  time = next - 1;
  await cycle({ ...deps }, AbortSignal.timeout(1000));
  assert.equal(analyses, 1);
  assert.equal(posts.length, 2);
  time = next;
  await cycle({ ...deps }, AbortSignal.timeout(1000));
  assert.equal(analyses, 2);
  assert.equal(posts.length, 2);
  assert.equal(store.snapshot.value.last_outcome, "duplicate");
  assert.equal(store.snapshot.value.lease_until, 0);
});

test("daily reviews retry startup failures promptly but preserve the full interval after any send attempt", async () => {
  const time = 1_800_000_000_000;
  const store = new MemoryState();
  const deps = {
    store,
    persist,
    sample: async (): Promise<Sample> => {
      throw new Error("server starting");
    },
    analyze: async () => ({ candidates: [verified], sha: "sha" }),
    post: async () => {
      throw new Error("delivery uncertain");
    },
    model: "model",
    dailyLimit: 144,
    intervalMinutes: 1440,
    maxFindings: 2,
    now: () => time,
  };
  await assert.rejects(
    cycle(deps, AbortSignal.timeout(1000)),
    /server starting/,
  );
  assert.equal(store.snapshot.value.next_at, time + 600_000);
  assert.equal(store.snapshot.value.posts, 0);
  const retry = time + 600_001;
  await assert.rejects(
    cycle(
      { ...deps, sample: async () => sample, now: () => retry },
      AbortSignal.timeout(1000),
    ),
    /delivery uncertain/,
  );
  assert.equal(store.snapshot.value.next_at, retry + 86_400_000);
  assert.equal(store.snapshot.value.posts, 1);
  assert.equal(store.snapshot.value.last_outcome, "delivery_unknown");
});

test("heartbeat loss cancels in-flight analysis and leaves evidence retryable", async (context) => {
  context.mock.timers.enable({
    apis: ["Date", "setTimeout"],
    now: 1_800_000_000_000,
  });
  const memory = new MemoryState();
  let writable = true;
  const store: StateStore<State> = {
    read: () => memory.read(),
    commit: (previous, value) => {
      if (!writable) return Promise.reject(new Error("state unavailable"));
      return memory.commit(previous, value);
    },
  };
  let started!: () => void;
  const analyzing = new Promise<void>((resolve) => {
    started = resolve;
  });
  const running = assert.rejects(
    cycle(
      {
        store,
        sample: async () => sample,
        analyze: async (_sample, _previous, signal) =>
          new Promise((_resolve, reject) => {
            signal.addEventListener(
              "abort",
              () => {
                writable = true;
                reject(signal.reason);
              },
              { once: true },
            );
            started();
          }),
        persist: async () =>
          assert.fail("Canceled analysis cannot persist a finding"),
        post: async () => assert.fail("Canceled analysis cannot post"),
        model: "fixture",
        dailyLimit: 12,
      },
      new AbortController().signal,
    ),
    /state unavailable/,
  );
  await analyzing;
  writable = false;
  context.mock.timers.tick(60_000);
  await running;
  assert.equal(memory.snapshot.value.lease_until, 0);
  assert.equal(memory.snapshot.value.signature, "");
  assert.equal(memory.snapshot.value.sent.length, 0);
  assert.equal(memory.snapshot.value.last_outcome, "failed");
});

test("renewal during native persistence preserves the delivery claim", async (context) => {
  context.mock.timers.enable({
    apis: ["Date", "setTimeout"],
    now: 1_800_000_000_000,
  });
  const store = new MemoryState();
  let posts = 0;
  await cycle(
    {
      store,
      sample: async () => sample,
      analyze: async () => ({ candidates: [verified], sha: "pinned" }),
      persist: async () => {
        for (let minute = 0; minute < 6; minute++) {
          context.mock.timers.tick(60_000);
          await setImmediate();
        }
        return native;
      },
      post: async () => {
        posts++;
      },
      model: "fixture",
      dailyLimit: 12,
    },
    new AbortController().signal,
  );
  assert.equal(posts, 1);
  assert.equal(store.snapshot.value.sent.length, 1);
  assert.equal(store.snapshot.value.lease_until, 0);
  assert.equal(store.snapshot.value.last_outcome, "delivered");
});

test("an expired worker claim resumes a daily review without bypassing an attempted delivery", async () => {
  const now = 1_800_000_000_000;
  for (const outcome of ["running", "delivery_unknown"] as const) {
    const store = new MemoryState();
    await store.commit(await store.read(), {
      ...initialState,
      owner: "stopped-worker",
      next_at: now + 86_400_000,
      lease_until: now - 1,
      last_outcome: outcome,
    });
    let posts = 0;
    await cycle(
      {
        store,
        sample: async () => sample,
        analyze: async () => ({ candidates: [verified], sha: "pinned" }),
        persist,
        post: async () => {
          posts++;
        },
        model: "fixture",
        dailyLimit: 12,
        intervalMinutes: 1440,
        now: () => now,
      },
      new AbortController().signal,
    );
    assert.equal(posts, outcome === "running" ? 1 : 0);
    assert.equal(store.snapshot.value.next_at, now + 86_400_000);
  }
});
