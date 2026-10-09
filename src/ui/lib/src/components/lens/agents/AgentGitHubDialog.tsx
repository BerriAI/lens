"use client";

import { useCallback, useEffect, useState } from "react";
import { Check, ExternalLink, Github, Loader2 } from "lucide-react";
import { Button } from "../../ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "../../ui/dialog";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../ui/select";
import type { EvalDefinition } from "../evals/runs/types";
import { GitHubEvalSetupDialog } from "./GitHubEvalSetupDialog";
import { gitHubAuthorizationDestination, openGitHubAuthorization, useGitHubConnection } from "./githubConnection";

interface AgentGitHubDialogProps {
  readonly agent: string;
  readonly authorizationId?: string | null;
  readonly definition?: EvalDefinition;
  readonly onOpenChange: (open: boolean) => void;
  readonly onOpenDatasets?: () => void;
  readonly onAuthorize?: (url: string) => void;
  readonly onAuthorizationComplete?: () => void;
}

export function AgentGitHubDialog({
  agent,
  authorizationId,
  definition,
  onOpenChange,
  onOpenDatasets,
  onAuthorize = openGitHubAuthorization,
  onAuthorizationComplete,
}: AgentGitHubDialogProps) {
  const github = useGitHubConnection(agent, authorizationId);
  const [repositoryId, setRepositoryId] = useState<string | null>(null);
  const [settingUpEvals, setSettingUpEvals] = useState(false);
  const [redirectError, setRedirectError] = useState<string | null>(null);
  const connection = github.status.data?.connection;
  const authorized = github.authorization.data;
  const repositories = authorized?.repositories ?? [];
  const selected = repositories.find((repo) => String(repo.id) === repositoryId) ?? repositories[0];
  const startAuthorization = github.start.mutate;
  const begin = useCallback(
    (install: boolean) => {
      setRedirectError(null);
      startAuthorization(install, {
        onSuccess: ({ authorization_url }) => {
          try {
            onAuthorize(gitHubAuthorizationDestination(authorization_url, github.status.data?.service_origin));
          } catch {
            setRedirectError("Could not open GitHub. Try again");
          }
        },
      });
    },
    [startAuthorization, onAuthorize, github.status.data?.service_origin],
  );

  useEffect(() => {
    if (authorizationId && (github.connect.isSuccess || (connection && authorized?.status === "connected")))
      onAuthorizationComplete?.();
  }, [authorizationId, github.connect.isSuccess, connection, authorized?.status, onAuthorizationComplete]);

  if (settingUpEvals && connection)
    return (
      <GitHubEvalSetupDialog
        agent={agent}
        repository={connection.repository}
        definition={definition}
        onOpenChange={onOpenChange}
        onOpenDatasets={onOpenDatasets}
      />
    );

  const error =
    redirectError || github.start.error?.message || github.connect.error?.message || github.disconnect.error?.message;
  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[calc(100dvh-2rem)] overflow-y-auto sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Github aria-hidden="true" className="size-5" /> Connect GitHub
          </DialogTitle>
          <DialogDescription>
            Connect <span className="font-medium text-foreground">{agent}</span> to a repository through the Lens GitHub
            App
          </DialogDescription>
        </DialogHeader>

        {github.status.isPending ? (
          <p role="status" className="flex items-center gap-2 py-8 text-sm">
            <Loader2 aria-hidden="true" className="size-4 animate-spin" /> Checking GitHub connection
          </p>
        ) : github.status.error ? (
          <div className="space-y-3">
            <p role="alert" className="text-sm text-destructive">
              Could not check the GitHub connection
            </p>
            <Button variant="outline" onClick={() => void github.status.refetch()}>
              Try again
            </Button>
          </div>
        ) : !github.status.data?.configured ? (
          <div className="space-y-4 rounded-lg border p-4 text-sm">
            <h3 className="font-medium">GitHub connection unavailable</h3>
            <p className="text-muted-foreground">
              The Lens GitHub service has not been enabled for this deployment. Your administrator can check the
              connection settings so you can install the Lens App and choose a repository
            </p>
            <a
              className="inline-flex items-center gap-1 underline underline-offset-4"
              href="https://github.com/BerriAI/lens/blob/main/docs/github-app.md"
              target="_blank"
              rel="noreferrer"
            >
              GitHub connection guide <ExternalLink aria-hidden="true" className="size-3" />
            </a>
            <Button variant="outline" onClick={() => void github.status.refetch()}>
              Check configuration
            </Button>
          </div>
        ) : connection && (!authorizationId || github.connect.isSuccess || authorized?.status === "connected") ? (
          <div className="space-y-4">
            <div className="space-y-2 rounded-lg border p-4">
              <h3 className="flex items-center gap-2 font-medium">
                <Check aria-hidden="true" className="size-4" /> GitHub connected
              </h3>
              <a
                className="inline-flex items-center gap-1 break-all text-sm underline underline-offset-4"
                href={`https://github.com/${connection.repository}`}
                target="_blank"
                rel="noreferrer"
              >
                {connection.repository} <ExternalLink aria-hidden="true" className="size-3 shrink-0" />
              </a>
              <p className="text-xs text-muted-foreground">
                Connected through the GitHub App. Repository access is limited to the installation you authorized
              </p>
            </div>
            {connection.available ? (
              <div className="space-y-3">
                <p className="text-sm text-muted-foreground">
                  Next, configure the eval that should run on pull requests. Connecting GitHub does not run an eval by
                  itself
                </p>
                <Button onClick={() => setSettingUpEvals(true)}>Set up PR evals</Button>
              </div>
            ) : (
              <div className="space-y-3">
                <p role="alert" className="text-sm text-destructive">
                  GitHub access needs attention. {connection.availability_error ||
                    "The App may have been removed, suspended, or lost access to this repository"}
                </p>
                <Button disabled={github.start.isPending} onClick={() => begin(true)}>
                  Restore GitHub access
                </Button>
                <Button variant="outline" disabled={github.status.isFetching} onClick={() => void github.status.refetch()}>
                  Check connection again
                </Button>
              </div>
            )}
            <div className="flex flex-wrap gap-2 border-t pt-4">
              <Button variant="outline" disabled={github.start.isPending} onClick={() => begin(true)}>
                Manage repository access
              </Button>
              <Button
                variant="ghost"
                disabled={github.disconnect.isPending}
                onClick={() => github.disconnect.mutate()}
              >
                Disconnect
              </Button>
            </div>
          </div>
        ) : authorizationId ? (
          <div className="space-y-4">
            {github.authorization.isPending || authorized?.status === "pending" ? (
              <p role="status" className="flex items-center gap-2 py-8 text-sm">
                <Loader2 aria-hidden="true" className="size-4 animate-spin" /> Completing GitHub authorization
              </p>
            ) : github.authorization.error || authorized?.status === "failed" ? (
              <>
                <p role="alert" className="text-sm text-destructive">
                  GitHub authorization was cancelled, expired, or could not be completed. Your agent is not connected
                </p>
                <Button disabled={github.start.isPending} onClick={() => begin(false)}>
                  Try GitHub again
                </Button>
              </>
            ) : repositories.length === 0 ? (
              <>
                <h3 className="font-medium">Give Lens access to a repository</h3>
                <p className="text-sm text-muted-foreground">
                  Install the Lens GitHub App and choose the repositories it can access. You’ll return here to link one
                  to {agent}
                </p>
                <Button disabled={github.start.isPending} onClick={() => begin(true)}>
                  <Github aria-hidden="true" className="size-4" /> Install GitHub App
                </Button>
                <Button variant="ghost" disabled={github.start.isPending} onClick={() => begin(false)}>
                  I already installed it
                </Button>
              </>
            ) : (
              <>
                <div className="space-y-1">
                  <h3 className="font-medium">Choose a repository</h3>
                  <p className="text-sm text-muted-foreground">
                    Only repositories available to you through the GitHub App are shown
                  </p>
                </div>
                <label htmlFor="github-repository" className="text-sm font-medium">
                  GitHub repository
                </label>
                <Select value={selected ? String(selected.id) : null} onValueChange={setRepositoryId}>
                  <SelectTrigger id="github-repository" className="w-full">
                    <SelectValue>{selected?.full_name}</SelectValue>
                  </SelectTrigger>
                  <SelectContent>
                    {repositories.map((repo) => (
                      <SelectItem key={repo.id} value={String(repo.id)}>
                        {repo.full_name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <Button
                  disabled={!selected || github.connect.isPending}
                  onClick={() => selected && github.connect.mutate(selected.id)}
                >
                  {github.connect.isPending && <Loader2 aria-hidden="true" className="size-4 animate-spin" />} Connect
                  repository
                </Button>
                <Button variant="ghost" disabled={github.start.isPending} onClick={() => begin(true)}>
                  Missing a repository? Manage App access
                </Button>
              </>
            )}
          </div>
        ) : (
          <div className="space-y-4 py-4">
            <p className="text-sm text-muted-foreground">
              Install the Lens App for your GitHub account or organization. If it is already installed, authorize
              Lens to choose a repository
            </p>
            <div className="flex flex-wrap gap-2">
              <Button disabled={github.start.isPending || github.status.isFetching} onClick={() => begin(true)}>
                {github.start.isPending ? <Loader2 aria-hidden="true" className="size-4 animate-spin" /> : <Github aria-hidden="true" className="size-4" />}
                Install GitHub App
              </Button>
              <Button variant="outline" disabled={github.start.isPending || github.status.isFetching} onClick={() => begin(false)}>
                I already installed it
              </Button>
            </div>
          </div>
        )}
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}
      </DialogContent>
    </Dialog>
  );
}
