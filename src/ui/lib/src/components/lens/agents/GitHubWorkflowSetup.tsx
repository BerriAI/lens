"use client";

import { useState, type ReactNode } from "react";
import { Tabs } from "@base-ui/react/tabs";
import { Check, ExternalLink, GitPullRequest, Loader2 } from "lucide-react";
import { Button } from "../../ui/button";
import { useEvalRuns } from "../evals/runs/api";
import {
  evalFilePath,
  evalSnippet,
  githubRunRepository,
  pyprojectSnippet,
  workflowSnippet,
  type ConnectTarget,
} from "../evals/runs/connectSnippets";
import type { EvalRun } from "../evals/runs/types";
import { CodeBlock } from "../onboarding/tracing/TracingSetupCard";
import { useEvalRunRoute } from "../route";

function GitHubLink({ href, children }: { href: string; children: ReactNode }) {
  return (
    <a
      href={href}
      target="_blank"
      rel="noreferrer"
      className="inline-flex items-center gap-1 text-sm underline underline-offset-4"
    >
      {children}
      <ExternalLink aria-hidden="true" className="size-3" />
    </a>
  );
}

export function WorkflowSetup({ target }: { target: ConnectTarget }) {
  const files = [
    { path: ".github/workflows/lens.yml", body: workflowSnippet(target) },
    { path: "pyproject.toml", body: pyprojectSnippet(target) },
    { path: evalFilePath(target), body: evalSnippet(target) },
  ];
  const [file, setFile] = useState(files[0].path);
  const shown = files.find((item) => item.path === file) ?? files[0];
  return (
    <div className="min-w-0 space-y-5 text-sm">
      <section className="space-y-3">
        <h3 className="font-medium">Add the workflow and eval adapter</h3>
        <p className="text-muted-foreground">
          Merge these files into your repository. Keep existing project settings and add the dependencies and startup
          steps your agent needs
        </p>
        <Tabs.Root value={file} onValueChange={setFile} className="min-w-0 overflow-hidden rounded-lg border">
          <Tabs.List aria-label="Setup files" className="flex overflow-x-auto border-b bg-muted/30">
            {files.map((item) => (
              <Tabs.Tab
                key={item.path}
                type="button"
                value={item.path}
                className={`whitespace-nowrap px-3 py-2 text-xs ${item.path === shown.path ? "bg-background font-medium" : "text-muted-foreground"}`}
              >
                {item.path}
              </Tabs.Tab>
            ))}
          </Tabs.List>
          <Tabs.Panel
            value={shown.path}
            aria-label={shown.path}
            className="min-w-0 [&_pre]:max-h-56 [&_pre]:overflow-auto"
          >
            <CodeBlock code={shown.body} copyLabel={`Copy ${shown.path}`} />
          </Tabs.Panel>
        </Tabs.Root>
        {!target.taskImport && (
          <p className="rounded-md border border-amber-500/30 bg-amber-500/5 p-3 text-xs leading-5">
            Implement task() in the eval adapter before running. It should start this commit’s agent in eval mode and
            return its trace reference. The template will fail until you connect your agent
          </p>
        )}
        <p className="text-xs leading-5 text-muted-foreground">
          Run the agent built from the checked-out commit, with the agent name{" "}
          <code>{target.definition.spec.agent}</code>, its build SHA as <code>agent.version</code>, and{" "}
          <code>deployment.environment=lens-eval</code> on its traces.{" "}
          <a
            className="underline"
            target="_blank"
            rel="noreferrer"
            href="https://github.com/BerriAI/lens/blob/main/src/sdk/README.md#complete-http-agent-example"
          >
            Agent adapter guide
          </a>
        </p>
      </section>
      <section className="space-y-3">
        <h3 className="font-medium">Add repository credentials</h3>
        <div className="rounded-lg border p-3 text-xs leading-6">
          <p>
            <code className="font-medium">LENS_API_KEY</code> secret: a Lens credential that can read datasets and run
            evals. A tracing key cannot run evals
          </p>
          <p>
            <code className="font-medium">LENS_BASE_URL</code> variable:{" "}
            <span className="break-all">{target.baseUrl}</span>
          </p>
          <p className="mt-2 text-muted-foreground">
            Map your agent’s model and service secrets under env on the Lens Action step and any agent startup step.{" "}
            {target.reportViaApp
              ? "Lens publishes comments and checks through the GitHub App you connected"
              : "The workflow uses GitHub’s built-in token for comments and checks"}
          </p>
        </div>
        <CodeBlock
          code={"env:\n  AGENT_API_KEY: ${{ secrets.AGENT_API_KEY }}"}
          copyLabel="Copy agent secret mapping example"
        />
        <div className="flex flex-wrap gap-4">
          <GitHubLink href={`${target.repository?.url}/settings/secrets/actions`}>Open repository secrets</GitHubLink>
          <GitHubLink href={`${target.repository?.url}/settings/variables/actions`}>
            Open repository variables
          </GitHubLink>
        </div>
        <p className="text-xs leading-5 text-muted-foreground">
          Your organization must allow this repository to use the Lens Action.{" "}
          <a
            className="underline"
            target="_blank"
            rel="noreferrer"
            href="https://docs.github.com/en/actions/how-tos/reuse-automations/share-with-your-organization"
          >
            Private Action access
          </a>
        </p>
      </section>
      <section className="space-y-2 border-t pt-4">
        <h3 className="font-medium">Run a baseline, then open a PR</h3>
        <p className="text-xs leading-5 text-muted-foreground">
          Commit to main or run the Lens workflow on main from Actions. Then open or update a PR from a branch in this
          repository. Fork PRs are skipped because they cannot safely use your credentials
        </p>
        <GitHubLink href={`${target.repository?.url}/actions/workflows/lens.yml`}>Open Lens workflow</GitHubLink>
      </section>
    </div>
  );
}

export function VerifyGitHub({
  target,
  runs,
  onOpenChange,
}: {
  target: ConnectTarget;
  runs: ReturnType<typeof useEvalRuns>;
  onOpenChange: (open: boolean) => void;
}) {
  const { openAgentRun } = useEvalRunRoute();
  const matching =
    runs.data?.filter(
      (run) =>
        run.agent === target.definition.spec.agent &&
        run.eval === target.definition.name &&
        githubRunRepository(run.ci_url)?.fullName.toLowerCase() === target.repository?.fullName.toLowerCase(),
    ) ?? [];
  const pull = matching.find((run) => run.pr !== null);
  const baseline = matching.find(
    (run) => run.pr === null && run.branch === target.definition.spec.baseline && run.status === "done",
  );
  const received = pull?.status === "done" && Boolean(pull.summary);
  return (
    <div className="space-y-4">
      <div className="rounded-xl border bg-muted/20 p-5 text-center">
        {received ? (
          <Check aria-hidden="true" className="mx-auto mb-3 size-7 text-success" />
        ) : (
          <GitPullRequest aria-hidden="true" className="mx-auto mb-3 size-7 text-muted-foreground" />
        )}
        <h3 className="font-medium">
          {received
            ? "PR eval received"
            : pull?.status === "failed"
              ? "PR eval needs attention"
              : "Waiting for your first PR eval"}
        </h3>
        <p className="mt-2 text-sm text-muted-foreground">
          {pull ? runStatus(pull) : "Open or update a PR in the selected repository, then check for its result here"}
        </p>
      </div>
      <p className="text-xs text-muted-foreground">
        {baseline
          ? "Baseline received from main"
          : "No completed main baseline received yet. Run the workflow on main to compare regressions"}
      </p>
      {runs.error && (
        <p role="alert" className="text-sm text-destructive">
          Could not check eval runs. Try again
        </p>
      )}
      <div className="flex flex-wrap items-center gap-3">
        <Button variant="outline" disabled={runs.isFetching} onClick={() => void runs.refetch()}>
          {runs.isFetching && <Loader2 aria-hidden="true" className="size-4 animate-spin" />} Check for PR eval
        </Button>
        {pull && (
          <Button
            onClick={() => {
              openAgentRun(target.definition.spec.agent, target.definition.name, pull.id);
              onOpenChange(false);
            }}
          >
            View eval result
          </Button>
        )}
        <GitHubLink href={`${target.repository?.url}/actions/workflows/lens.yml`}>View workflow</GitHubLink>
        {pull && <GitHubLink href={`${target.repository?.url}/pull/${pull.pr}`}>Open PR #{pull.pr}</GitHubLink>}
      </div>
      <div className="space-y-2 rounded-lg border p-4 text-xs leading-5 text-muted-foreground">
        <p className="font-medium text-foreground">What appears on your PR</p>
        <p>
          Lens posts the eval result, pass counts, regressions and a link to the full run.{" "}
          {target.reportViaApp
            ? "The connected GitHub App posts a report for each eval run. Retrying publication updates that run’s report"
            : "New commits update the same comment"}
        </p>
        <p>
          After a result arrives here, check the workflow’s Publish report step and the PR for its comment. Lens
          receives the eval before GitHub publishes it
        </p>
      </div>
    </div>
  );
}

function runStatus(run: EvalRun): string {
  if (run.status === "failed")
    return run.failure || "The eval failed to finish. Open its result or workflow logs for details";
  if (run.status !== "done" || !run.summary)
    return `PR #${run.pr} is ${run.status}. Check again when the workflow finishes`;
  return `PR #${run.pr}: ${run.summary.passed}/${run.summary.total} cases passed. ${run.summary.gate.passed ? "Gate passed" : "Gate failed, review the result before merging"}`;
}
