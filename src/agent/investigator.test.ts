import assert from "node:assert/strict";
import { test } from "node:test";
import {
  bootstrapKnownFindings,
  cycle,
  initialState,
  type State,
} from "./investigator.js";
import { digest, type Snapshot, type StateStore } from "./state.js";
import { Evidence, type Sample } from "./evidence.js";
import { verify, type Candidate, type VerifiedCandidate } from "./findings.js";
import { LensClient } from "./lens.js";
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
      analyze: async () => ({ candidate: verified, sha: "sha" }),
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
      return { candidate: verified, sha: "pinned" };
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
      return { candidate: verified, sha: "pinned" };
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
      return { sha: "pinned" };
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
      candidate: {
        ...verified,
        fingerprint: posts ? "b".repeat(64) : firstFingerprint,
        evidenceHash: String(time),
      },
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
    analyze: async () => ({ candidate: verified, sha: "pinned" }),
    post: async () => {
      attempts++;
      throw new Error("lost Slack response");
    },
    model: "model",
    dailyLimit: 12,
    now: () => time,
  };
  await assert.rejects(cycle(deps, AbortSignal.timeout(1000)));
  time += 600_001;
  await cycle(deps, AbortSignal.timeout(1000));
  assert.equal(attempts, 1);
  assert.equal(store.snapshot.value.posts, 1);
});

test("expired ownership cannot post after a long model request", async () => {
  const store = new MemoryState();
  let time = 1_800_000_000_000;
  let posts = 0;
  await cycle(
    {
      store,
      persist,
      sample: async () => sample,
      analyze: async () => {
        time += 300_000;
        return { candidate: verified, sha: "pinned" };
      },
      post: async () => {
        posts++;
      },
      model: "model",
      dailyLimit: 12,
      now: () => time,
    },
    AbortSignal.timeout(1000),
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
      return { sha: "pinned" };
    },
    post: async () => {},
    model: "model",
    dailyLimit: 12,
    now: () => time,
  };
  await assert.rejects(cycle(deps, AbortSignal.timeout(1000)));
  assert.equal(store.snapshot.value.signature, "");
  time += 600_001;
  await cycle(deps, AbortSignal.timeout(1000));
  assert.equal(attempts, 2);
  assert.equal(store.snapshot.value.runs, 2);
  assert.notEqual(store.snapshot.value.signature, "");
});

test("native import failures do not reserve a Slack send and retry the same verified identity", async () => {
  const store = new MemoryState();
  let time = 1_800_000_000_000;
  let imports = 0;
  let posts = 0;
  const deps = {
    store,
    sample: async () => sample,
    analyze: async () => ({ candidate: verified, sha: "pinned" }),
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
  await cycle(
    {
      store,
      sample: async () => sample,
      analyze: async () => ({ candidate: verified, sha: "pinned" }),
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
