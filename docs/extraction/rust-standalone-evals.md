# Standalone evaluation qualification

The local Rust candidate completed an evaluation against an actual paid model, saved its results in ClickHouse, and exposed the failed gate and original traces in the standalone UI. This qualifies the observed evaluation workflow, not the complete independent release

The dataset contains one arithmetic case. The baseline trace answered that two plus two is four, and the candidate answered five. Lens ran the configured judge and task-completion scorers against the saved trace content. The baseline passed, the candidate failed, and the comparison reported one regression with a failed gate. Both runs used the same frozen dataset revision and scorer configuration

The [sanitized API evidence](evidence/rust-live-eval.json) records the requests, persisted details and comparison links. The latest candidate is `c27be4d9643545989795b80f94495d23`, compared with baseline `3c7e6b67f6374955bba41b66dd7c077c`. Provider credentials and ingestion keys are excluded

Opening the run in the browser showed the failed gate and comparison. **Open trace** opened the original candidate trace and its exact answer, “Two plus two is five.” The links preserve the dataset, run and case selection and carry the actual stored trace identity. Baseline case links also work when that case did not change in the baseline run

![Failed evaluation gate and comparison](evidence/ui/standalone-eval/failed-gate.png)

The provider connection used the configured hosted model endpoint. No local LiteLLM gateway or Postgres instance was required by Lens. This proof does not establish native direct-provider credentials, which were unavailable in the approved test environment

Focused ClickHouse and HTTP tests cover frozen revisions and cases, role checks, retries after storage failure, competing schedulers, duplicate receipts, missing trials, actual judge input, real gateway spend fallback, explicit SDK costs, persistent trace references and compatible baseline selection. The UI tests consume a recorded Rust response. Contract-version checks reject incompatible requests before mutations

The remaining eval qualification includes finishing the mutation campaign and the independent SDK, embedded gateway and release acceptance checks. Those are tracked in the implementation ledger; the live proof alone does not close them
