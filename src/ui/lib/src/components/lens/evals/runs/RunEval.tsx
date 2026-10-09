"use client";

import { useEffect, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { ExternalLink, Github, Loader2, Play } from "lucide-react";
import { apiClient } from "../../../../lib/http/requests";
import { Button } from "../../../ui/button";
import { AgentGitHubDialog } from "../../agents/AgentGitHubDialog";
import { useGitHubConnection } from "../../agents/githubConnection";
import { useLensAccessToken, useLensApi } from "../../data/LensServices";
import { useOptionalOnboarding } from "../../onboarding/OnboardingContext";
import { githubRunRepository } from "./connectSnippets";
import { shortSha } from "./format";
import type { EvalDefinition, EvalRun } from "./types";

interface RunEvalProps {
  readonly definition: EvalDefinition;
  readonly runs: readonly EvalRun[];
  readonly onRefresh: () => Promise<unknown>;
}

interface RerunResponse {
  readonly ci_url: string;
  readonly requested_at: string;
}

export function RunEval(props: RunEvalProps) {
  const { scope } = useLensApi();
  if (scope === "demo") return null;
  return <LiveRunEval {...props} />;
}

function LiveRunEval({ definition, runs, onRefresh }: RunEvalProps) {
  const accessToken = useLensAccessToken();
  const github = useGitHubConnection(definition.spec.agent);
  const onboarding = useOptionalOnboarding();
  const [connecting, setConnecting] = useState(false);
  const [waitExpired, setWaitExpired] = useState(false);
  const [previousIds, setPreviousIds] = useState<ReadonlySet<string> | null>(
    null,
  );
  const connection = github.status.data?.connection;
  const source =
    connection &&
    (runs.find(
      (run) =>
        githubRunRepository(run.ci_url)?.fullName.toLowerCase() ===
        connection.repository.toLowerCase(),
    ) ?? runs.find((run) => githubRunRepository(run.ci_url)));
  const rerun = useMutation({
    mutationFn: (runId: string) =>
      apiClient.post<RerunResponse>("/lens/github/rerun", {
        accessToken,
        headers: { "X-Lens-Contract": "1" },
        body: { run_id: runId },
      }),
    retry: false,
    onSuccess: () => {
      setWaitExpired(false);
      setPreviousIds(new Set(runs.map((run) => run.id)));
      void onRefresh();
    },
  });
  const received =
    previousIds &&
    runs.some(
      (run) =>
        !previousIds.has(run.id) &&
        run.ci_url?.replace(/\/attempts\/\d+\/?$/, "") ===
          rerun.data?.ci_url.replace(/\/attempts\/\d+\/?$/, ""),
    );
  const waiting = rerun.isSuccess && !received && !waitExpired;
  useEffect(() => {
    if (!waiting) return;
    const timer = window.setInterval(() => void onRefresh(), 5000);
    return () => window.clearInterval(timer);
  }, [waiting, onRefresh]);
  useEffect(() => {
    if (!waiting) return;
    const timeout = window.setTimeout(() => setWaitExpired(true), 300_000);
    return () => window.clearTimeout(timeout);
  }, [waiting]);
  const canRun = connection?.available && source && !onboarding?.readOnly;
  return (
    <section aria-label="Run eval" className="shrink-0 border-b px-6 py-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="max-w-2xl space-y-1">
          <h2 className="text-sm font-medium">
            {definition.spec.agent_io
              ? "Run your saved agent contract"
              : "Run your agent code"}
          </h2>
          <p className="text-xs leading-5 text-muted-foreground">
            {source
              ? `Reruns the workflow for ${source.branch}@${shortSha(source.version)} in GitHub Actions. The workflow controls the dataset revision and may run other evals`
              : "Lens stores the test cases, scores and traces. Your project runs the agent locally or in GitHub Actions"}
          </p>
        </div>
        {canRun ? (
          <Button
            size="sm"
            disabled={
              rerun.isPending ||
              waiting ||
              source.status === "running" ||
              source.status === "scoring"
            }
            onClick={() => rerun.mutate(source.id)}
          >
            {rerun.isPending || waiting ? (
              <Loader2 aria-hidden="true" className="size-4 animate-spin" />
            ) : (
              <Play aria-hidden="true" className="size-4" />
            )}
            {waiting ? "Run requested" : "Run again"}
          </Button>
        ) : (
          !onboarding?.readOnly &&
          !github.status.isPending && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => setConnecting(true)}
            >
              <Github aria-hidden="true" className="size-4" /> Set up execution
            </Button>
          )
        )}
      </div>
      {github.status.error && (
        <p role="alert" className="mt-2 text-xs text-destructive">
          Could not check GitHub access. {github.status.error.message}
        </p>
      )}
      {connection && !connection.available && (
        <p role="alert" className="mt-2 text-xs text-destructive">
          GitHub access needs attention.{" "}
          {connection.availability_error ||
            "Reconnect the repository to run this eval"}
        </p>
      )}
      {rerun.error && (
        <p role="alert" className="mt-2 text-xs text-destructive">
          Could not start the run. {rerun.error.message}
        </p>
      )}
      {rerun.data && (
        <p
          role="status"
          className="mt-3 flex flex-wrap items-center gap-2 text-xs text-muted-foreground"
        >
          {received
            ? "New results received. Open the run below to inspect each case"
            : waitExpired
              ? "No new results received after 5 minutes. Check the workflow for failures or missing Lens reporting"
              : "GitHub accepted the request. Waiting for this workflow to send results to Lens"}
          {githubRunRepository(rerun.data.ci_url) && (
            <a
              className="inline-flex items-center gap-1 underline underline-offset-4"
              href={rerun.data.ci_url}
              target="_blank"
              rel="noreferrer"
            >
              Open workflow{" "}
              <ExternalLink aria-hidden="true" className="size-3" />
            </a>
          )}
        </p>
      )}
      {connecting && (
        <AgentGitHubDialog
          agent={definition.spec.agent}
          definition={definition}
          onOpenChange={setConnecting}
        />
      )}
    </section>
  );
}
