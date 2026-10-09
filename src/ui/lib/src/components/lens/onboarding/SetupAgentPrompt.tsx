"use client";

import { useEffect, useState } from "react";
import { Check, Copy } from "lucide-react";
import { useTimeout } from "usehooks-ts";
import { Button } from "../../ui/button";
import { copyToClipboard } from "../../../utils/dataUtils";
import { useLensHost } from "../../../host/LensHost";
import { getRequestBaseUrl } from "../../../lib/http/runtime";
import {
  setupPrompt,
  type SetupConnection,
  type SetupEval,
  type SetupGoal,
} from "./setupPrompt";

export function SetupAgentPrompt({
  goal = "start",
  connection,
  evaluation,
  featureConfigured,
  agentName,
  prominent = false,
}: {
  readonly goal?: SetupGoal;
  readonly connection?: SetupConnection;
  readonly evaluation?: SetupEval;
  readonly featureConfigured?: boolean;
  readonly agentName?: string;
  readonly prominent?: boolean;
}) {
  const standalone = useLensHost().surface === "standalone";
  const [baseUrl, setBaseUrl] = useState("");
  const [copied, setCopied] = useState(false);
  useTimeout(() => setCopied(false), copied ? 1600 : null);
  const currentBaseUrl = () => getRequestBaseUrl() || window.location.origin;
  useEffect(() => {
    setBaseUrl(getRequestBaseUrl() || window.location.origin);
  }, []);
  const context = {
    standalone,
    goal,
    connection,
    evaluation,
    featureConfigured,
    agentName,
  };
  const prompt = setupPrompt({ ...context, baseUrl });
  const copy = async () => {
    const current = currentBaseUrl();
    setBaseUrl(current);
    setCopied(
      await copyToClipboard(
        setupPrompt({ ...context, baseUrl: current }),
        "Prompt copied",
      ),
    );
  };
  return (
    <div
      className={
        prominent
          ? "w-full rounded-lg border bg-card p-5 text-sm sm:p-6"
          : "my-4 w-full rounded-lg border bg-muted/20 p-4 text-sm"
      }
    >
      {prominent && (
        <div className="mb-5 space-y-4">
          <div>
            <h3 className="text-base font-medium">
              Give this to your coding agent
            </h3>
            <p className="mt-2 text-sm leading-6 text-muted-foreground">
              Paste into Codex, Claude Code, or the coding agent working in your
              project.
            </p>
          </div>
          <blockquote className="rounded-md bg-muted/40 p-4 text-sm leading-7">
            Connect this project to Lens. Send a trace and verify that Lens
            received it. Then ask me, “What would you like to instrument next?”
            and help me do that.
          </blockquote>
        </div>
      )}
      <div className="flex flex-wrap items-center gap-3">
        <Button
          variant={prominent ? "default" : "outline"}
          size={prominent ? "default" : "sm"}
          aria-label={
            prominent ? "Copy setup instructions" : "Set it up for me"
          }
          onClick={() => void copy()}
        >
          {copied ? (
            <Check aria-hidden="true" className="size-3.5" />
          ) : (
            <Copy aria-hidden="true" className="size-3.5" />
          )}
          {copied
            ? "Prompt copied"
            : prominent
              ? "Copy setup instructions"
              : "Set it up for me"}
        </Button>
        <p className="text-xs leading-5 text-muted-foreground">
          {prominent
            ? "Includes your Lens address. No keys or secrets."
            : "Copy a prompt for your coding agent. It checks your setup and asks only for missing choices."}
        </p>
      </div>
      <details
        className="mt-2 text-xs text-muted-foreground"
        onToggle={(event) => {
          if (event.currentTarget.open) setBaseUrl(currentBaseUrl());
        }}
      >
        <summary className="w-fit cursor-pointer">
          {prominent ? "View full instructions" : "View prompt"}
        </summary>
        <pre className="mt-3 max-h-64 overflow-auto whitespace-pre-wrap break-words font-sans leading-5">
          {prompt}
        </pre>
      </details>
    </div>
  );
}
