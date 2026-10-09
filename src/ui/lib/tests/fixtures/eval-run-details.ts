import type { RunDetails } from "../../src/components/lens/datasets/runs/contract";

export default {
  run: {
    id: "c27be4d9643545989795b80f94495d23",
    status: "done",
    eval: "rust-live-proof",
    agent: "eval-proof-agent",
    version: "proof-candidate",
    branch: "arithmetic-regression",
    pr: 42,
    url: "http://127.0.0.1:4324/ui/?tab=datasets&dataset=656b5a25-0363-45e7-b15f-994e4911fdd2&dataset_tab=runs&eval_run=c27be4d9643545989795b80f94495d23",
    expected_trials: 1,
    received_trials: 1,
    summary: {
      passed: 0,
      total: 1,
      pass_rate: 0,
      cost_per_case: 0.001,
      scores: {
        judge: 0,
        task_completed: 1,
      },
      errors: 0,
      baseline_run_id: "3c7e6b67f6374955bba41b66dd7c077c",
      baseline_version: "proof-main",
      regressions: [
        {
          case_id:
            "b88ca87f107696ba8f50481adf5765b369dd0194ac0f6679908deea536f0e88c",
          title:
            "b88ca87f107696ba8f50481adf5765b369dd0194ac0f6679908deea536f0e88c",
          critical: false,
          baseline_url:
            "http://127.0.0.1:4324/ui/?tab=datasets&dataset=656b5a25-0363-45e7-b15f-994e4911fdd2&dataset_tab=runs&eval_run=3c7e6b67f6374955bba41b66dd7c077c&eval_case=b88ca87f107696ba8f50481adf5765b369dd0194ac0f6679908deea536f0e88c",
          candidate_url:
            "http://127.0.0.1:4324/ui/?tab=datasets&dataset=656b5a25-0363-45e7-b15f-994e4911fdd2&dataset_tab=runs&eval_run=c27be4d9643545989795b80f94495d23&eval_case=b88ca87f107696ba8f50481adf5765b369dd0194ac0f6679908deea536f0e88c",
        },
      ],
      fixed: [],
      gate: {
        passed: false,
        reasons: ["1 regressions (max 0)"],
      },
    },
    failure: "",
  },
  dataset_id: "656b5a25-0363-45e7-b15f-994e4911fdd2",
  dataset_revision: 1,
  created_at: "2026-10-09T04:18:21.065630Z",
  completed_at: "2026-10-09T04:18:23.263641Z",
  ci_url: "",
  cases: [
    {
      case_id:
        "b88ca87f107696ba8f50481adf5765b369dd0194ac0f6679908deea536f0e88c",
      input: "What is two plus two?",
      verdict: false,
      traces: [
        {
          trace_id: "13431fa547f1bffa2bf17d1257294add",
          trace_ref:
            "943E79520A9ECC824FB1FF5D498388E293A37A0C3C2CFFE5E650473325BCA35F",
        },
      ],
    },
  ],
  baseline: {
    run: {
      id: "3c7e6b67f6374955bba41b66dd7c077c",
      status: "done",
      eval: "rust-live-proof",
      agent: "eval-proof-agent",
      version: "proof-main",
      branch: "main",
      pr: null,
      url: "http://127.0.0.1:4324/ui/?tab=datasets&dataset=656b5a25-0363-45e7-b15f-994e4911fdd2&dataset_tab=runs&eval_run=3c7e6b67f6374955bba41b66dd7c077c",
      expected_trials: 1,
      received_trials: 1,
      summary: {
        passed: 1,
        total: 1,
        pass_rate: 1,
        cost_per_case: 0.001,
        scores: {
          judge: 1,
          task_completed: 1,
        },
        errors: 0,
        baseline_run_id: null,
        baseline_version: null,
        regressions: [],
        fixed: [],
        gate: {
          passed: true,
          reasons: ["no baseline on main for rev 1"],
        },
      },
      failure: "",
    },
    cases: [
      {
        case_id:
          "b88ca87f107696ba8f50481adf5765b369dd0194ac0f6679908deea536f0e88c",
        input: "What is two plus two?",
        verdict: true,
        traces: [
          {
            trace_id: "19e92c5911f4f5b7d7a7862d093f89f8",
            trace_ref:
              "BE2A81FD5C4B242F5F624CF1BEBBCB86D93EC6F54A71A1DA147B28E8C5BF920B",
          },
        ],
      },
    ],
  },
} as const satisfies RunDetails;
