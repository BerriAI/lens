"use client";

import { useState } from "react";
import { Loader2 } from "lucide-react";

import CopyButton from "../../../shared/CopyButton";
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

export interface ConnectAgentProps {
  readonly agent: string;
  readonly dataset: string;
  readonly revision: number;
}

function lensBaseUrl() {
  const base = getRequestBaseUrl();
  if (base) return base.replace(/\/+$/, "");
  return typeof window === "undefined" ? "" : window.location.origin;
}

export function ConnectAgent({ agent, dataset, revision }: ConnectAgentProps) {
  const target: ConnectTarget = {
    agent,
    dataset,
    revision,
    baseUrl: lensBaseUrl(),
  };
  const files = [
    { path: ".github/workflows/lens.yml", body: workflowSnippet(target) },
    { path: "pyproject.toml", body: pyprojectSnippet(target) },
    { path: `evals/${evalModule(agent)}.py`, body: evalSnippet(target) },
  ];
  const [file, setFile] = useState(files[0].path);
  const shown = files.find((item) => item.path === file) ?? files[0];
  const install = githubAppInstallUrl(process.env.NEXT_PUBLIC_LENS_GITHUB_APP);
  return (
    <section
      aria-label="Connect agent"
      className="mx-auto max-w-4xl px-4 py-5 text-sm"
    >
      <header className="mb-4 flex items-baseline justify-between gap-4 border-b pb-3">
        <div>
          <h3 className="font-semibold">Gate {agent} on every PR</h3>
          <p className="mt-0.5 text-xs text-muted-foreground">
            CI replays{" "}
            <span className="font-mono text-foreground">
              {dataset}@{revision}
            </span>{" "}
            against each commit and Lens fails the check when a case that passes
            on main breaks.
          </p>
        </div>
      </header>
      <ol className="divide-y rounded-md border">
        <Step n={1} title="Agent and dataset" done>
          <dl className="grid grid-cols-[6rem_1fr] gap-y-1 font-mono text-xs">
            <dt className="text-muted-foreground">agent</dt>
            <dd>{agent}</dd>
            <dt className="text-muted-foreground">dataset</dt>
            <dd>
              {dataset}@{revision}
            </dd>
            <dt className="text-muted-foreground">lens</dt>
            <dd>{target.baseUrl}</dd>
          </dl>
        </Step>
        <Step n={2} title="Connect GitHub">
          <div className="flex flex-wrap items-center gap-3">
            {install ? (
              <a
                href={install}
                target="_blank"
                rel="noreferrer"
                className="inline-flex items-center gap-2 rounded-md border bg-foreground px-3 py-1.5 text-background hover:opacity-90 focus-visible:outline-2 focus-visible:outline-ring"
              >
                <LensBrand className="w-16 rounded-sm bg-background px-1 py-0.5" />
                Install the Lens GitHub App
              </a>
            ) : (
              <span className="inline-flex items-center gap-2 rounded-md border border-dashed px-3 py-1.5 text-muted-foreground">
                <LensBrand className="w-16 opacity-60" />
                GitHub App not configured on this deployment
              </span>
            )}
            <span className="text-xs text-muted-foreground">
              or commit these files. Add{" "}
              <code className="font-mono text-foreground">LENS_API_KEY</code>{" "}
              and{" "}
              <code className="font-mono text-foreground">LENS_SDK_TOKEN</code>{" "}
              as repo secrets.
            </span>
          </div>
          <div className="mt-3 overflow-hidden rounded-md border">
            <div
              role="tablist"
              aria-label="Setup files"
              className="flex border-b bg-muted/30 font-mono text-xs"
            >
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
        </Step>
        <Step n={3} title="Waiting for the first CI run">
          <p
            role="status"
            className="flex items-center gap-2 font-mono text-xs text-muted-foreground"
          >
            <Loader2
              aria-hidden="true"
              className="size-3.5 animate-spin motion-reduce:animate-none"
            />
            listening for runs from {agent} on {dataset}. Push to main to record
            the baseline.
          </p>
        </Step>
      </ol>
    </section>
  );
}

function Step({
  n,
  title,
  done = false,
  children,
}: {
  n: number;
  title: string;
  done?: boolean;
  children: React.ReactNode;
}) {
  return (
    <li className="grid grid-cols-[1.5rem_1fr] gap-3 px-4 py-3">
      <span
        aria-hidden="true"
        className={cn(
          "mt-0.5 flex size-5 items-center justify-center rounded-[3px] border font-mono text-[11px]",
          done ? "border-success/50 text-success" : "text-muted-foreground",
        )}
      >
        {done ? "✓" : n}
      </span>
      <div className="min-w-0">
        <h4 className="mb-2 text-xs font-medium tracking-wide text-muted-foreground uppercase">
          {title}
        </h4>
        {children}
      </div>
    </li>
  );
}
