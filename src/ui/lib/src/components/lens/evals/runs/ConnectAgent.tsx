"use client";

import { useState } from "react";
import { Loader2 } from "lucide-react";

import CopyButton from "../../../shared/CopyButton";
import { Button } from "../../../ui/button";
import { cn } from "../../../../lib/cva.config";
import { getRequestBaseUrl } from "../../../../lib/http/runtime";
import { LensBrand } from "../../LensBrand";
import {
  evalModule,
  evalSnippet,
  githubAppInstallUrl,
  pyprojectSnippet,
  workflowSnippet,
  type ConnectTarget,
} from "./connectSnippets";
import type { EvalDefinition } from "./types";
import { SetupAgentPrompt } from "../../onboarding/SetupAgentPrompt";

export interface ConnectAgentProps {
  readonly definition: EvalDefinition;
  readonly dataset: string;
  readonly revision: number;
}

function lensBaseUrl() {
  const base = getRequestBaseUrl();
  if (base) return base.replace(/\/+$/, "");
  return typeof window === "undefined" ? "" : window.location.origin;
}

export function ConnectAgent({ definition, dataset, revision }: ConnectAgentProps) {
  const [manual, setManual] = useState(false);
  const target: ConnectTarget = { definition, dataset, revision, baseUrl: lensBaseUrl() };
  const install = githubAppInstallUrl(process.env.NEXT_PUBLIC_LENS_GITHUB_APP);
  return (
    <section aria-label="Connect agent" className="mx-auto flex max-w-3xl flex-col items-center gap-4 px-4 py-10 text-sm">
      <div className="text-center">
        <h3 className="font-semibold">No runs yet</h3>
        <p className="mt-1 text-xs text-muted-foreground">
          Connect the repo for <span className="font-mono text-foreground">{definition.spec.agent}</span> and Lens runs{" "}
          <span className="font-mono text-foreground">{definition.name}</span> on every PR.
        </p>
      </div>
      <SetupAgentPrompt
        goal="evals"
        evaluation={{ name: definition.name, agent: definition.spec.agent, dataset, revision }}
      />
      <div className="flex items-center gap-2">
        {install ? (
          <Button
            nativeButton={false}
            render={<a href={install} target="_blank" rel="noreferrer" />}
            className="gap-2"
          >
            <LensBrand className="w-14 rounded-sm bg-background px-1 py-0.5" />
            Connect to GitHub
          </Button>
        ) : (
          <Button disabled className="gap-2" title="Set NEXT_PUBLIC_LENS_GITHUB_APP to enable">
            <LensBrand className="w-14 rounded-sm bg-background px-1 py-0.5" />
            Connect to GitHub
          </Button>
        )}
        <Button variant="outline" aria-expanded={manual} onClick={() => setManual((open) => !open)}>
          Set up manually
        </Button>
      </div>
      {!install && (
        <p className="text-xs text-muted-foreground">The Lens GitHub App isn&apos;t configured on this deployment.</p>
      )}
      {manual && <ManualSetup target={target} />}
      <p role="status" className="flex items-center gap-2 font-mono text-xs text-muted-foreground">
        <Loader2 aria-hidden="true" className="size-3.5 animate-spin motion-reduce:animate-none" />
        waiting for the first run of {definition.name}
      </p>
    </section>
  );
}

function ManualSetup({ target }: { target: ConnectTarget }) {
  const files = [
    { path: ".github/workflows/lens.yml", body: workflowSnippet(target) },
    { path: "pyproject.toml", body: pyprojectSnippet(target) },
    { path: `evals/${evalModule(target.definition.name)}.py`, body: evalSnippet(target) },
  ];
  const [file, setFile] = useState(files[0].path);
  const shown = files.find((item) => item.path === file) ?? files[0];
  return (
    <div className="w-full">
      <p className="mb-2 text-xs text-muted-foreground">
        Commit these files and add <code className="font-mono text-foreground">LENS_API_KEY</code> and{" "}
        <code className="font-mono text-foreground">LENS_SDK_TOKEN</code> as repo secrets.
      </p>
      <div className="overflow-hidden rounded-md border">
        <div role="tablist" aria-label="Setup files" className="flex border-b bg-muted/30 font-mono text-xs">
          {files.map((item) => (
            <button
              key={item.path}
              type="button"
              role="tab"
              aria-selected={item.path === shown.path}
              onClick={() => setFile(item.path)}
              className={cn(
                "border-r px-3 py-1.5 text-muted-foreground hover:text-foreground",
                item.path === shown.path && "bg-background text-foreground",
              )}
            >
              {item.path}
            </button>
          ))}
          <span className="ml-auto flex items-center pr-1">
            <CopyButton value={shown.body} label={`Copy ${shown.path}`} />
          </span>
        </div>
        <pre
          role="tabpanel"
          aria-label={shown.path}
          className="max-h-72 overflow-auto bg-background p-3 font-mono text-xs leading-5"
        >
          {shown.body}
        </pre>
      </div>
    </div>
  );
}
