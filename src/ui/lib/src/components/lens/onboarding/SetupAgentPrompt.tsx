"use client";

import { useEffect, useState } from "react";
import { Check, Copy } from "lucide-react";
import { useTimeout } from "usehooks-ts";
import { Button } from "../../ui/button";
import { copyToClipboard } from "../../../utils/dataUtils";
import { useLensHost } from "../../../host/LensHost";
import { getRequestBaseUrl } from "../../../lib/http/runtime";
import { setupPrompt, type SetupConnection, type SetupEval, type SetupGoal } from "./setupPrompt";

export function SetupAgentPrompt({
  goal = "start",
  connection,
  evaluation,
  featureConfigured,
}: {
  readonly goal?: SetupGoal;
  readonly connection?: SetupConnection;
  readonly evaluation?: SetupEval;
  readonly featureConfigured?: boolean;
}) {
  const standalone = useLensHost().surface === "standalone";
  const [baseUrl, setBaseUrl] = useState("");
  const [copied, setCopied] = useState(false);
  useTimeout(() => setCopied(false), copied ? 1600 : null);
  const currentBaseUrl = () => getRequestBaseUrl() || window.location.origin;
  useEffect(() => {
    setBaseUrl(getRequestBaseUrl() || window.location.origin);
  }, []);
  const context = { standalone, goal, connection, evaluation, featureConfigured };
  const prompt = setupPrompt({ ...context, baseUrl });
  const copy = async () => {
    const current = currentBaseUrl();
    setBaseUrl(current);
    setCopied(await copyToClipboard(setupPrompt({ ...context, baseUrl: current }), "Prompt copied"));
  };
  return (
    <div className="my-4 w-full rounded-lg border bg-muted/20 p-4 text-sm">
      <div className="flex flex-wrap items-center gap-3">
        <Button variant="outline" size="sm" aria-label="Set it up for me" onClick={() => void copy()}>
          {copied ? <Check aria-hidden="true" className="size-3.5" /> : <Copy aria-hidden="true" className="size-3.5" />}
          {copied ? "Prompt copied" : "Set it up for me"}
        </Button>
        <p className="text-xs leading-5 text-muted-foreground">
          Copy a prompt for your coding agent. It checks your setup and asks only for missing choices.
        </p>
      </div>
      <details
        className="mt-2 text-xs text-muted-foreground"
        onToggle={(event) => { if (event.currentTarget.open) setBaseUrl(currentBaseUrl()); }}
      >
        <summary className="w-fit cursor-pointer">View prompt</summary>
        <pre className="mt-3 max-h-64 overflow-auto whitespace-pre-wrap break-words font-sans leading-5">{prompt}</pre>
      </details>
    </div>
  );
}
