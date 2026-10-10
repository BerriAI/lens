"use client";

import { useEffect, useRef, useState } from "react";
import { Check, Copy, Loader2 } from "lucide-react";
import { useTimeout } from "usehooks-ts";
import { Button } from "../../ui/button";
import { copyToClipboard } from "../../../utils/dataUtils";
import { useLensHost } from "../../../host/LensHost";
import { getRequestBaseUrl } from "../../../lib/http/runtime";
import claudeCodeLogo from "../../../../public/assets/logos/claude-code.svg";
import codexLogo from "../../../../public/assets/logos/codex.svg";
import cursorLogo from "../../../../public/assets/logos/cursor.svg";
import { maskSecret } from "./tracing/tracingKeys";
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
  tracingKey,
  prepareTracingKey,
  onCopied,
  primaryAction = true,
}: {
  readonly goal?: SetupGoal;
  readonly connection?: SetupConnection;
  readonly evaluation?: SetupEval;
  readonly featureConfigured?: boolean;
  readonly agentName?: string;
  readonly prominent?: boolean;
  readonly tracingKey?: string;
  readonly prepareTracingKey?: () => Promise<string>;
  readonly onCopied?: () => void;
  readonly primaryAction?: boolean;
}) {
  const standalone = useLensHost().surface === "standalone";
  const [baseUrl, setBaseUrl] = useState("");
  const [copied, setCopied] = useState(false);
  const [copying, setCopying] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const copyButton = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (tracingKey && prominent) copyButton.current?.focus();
  }, [tracingKey, prominent]);
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
  const prompt = setupPrompt({
    ...context,
    baseUrl,
    tracingKey: tracingKey
      ? maskSecret(tracingKey)
      : prepareTracingKey
        ? "<created when you copy>"
        : undefined,
  });
  const copy = async (includeKey = true) => {
    if (copying) return;
    setCopying(true);
    setError(null);
    try {
      const key = includeKey
        ? tracingKey ?? (await prepareTracingKey?.())
        : undefined;
      const current = currentBaseUrl();
      setBaseUrl(current);
      const success = await copyToClipboard(
        setupPrompt({ ...context, baseUrl: current, tracingKey: key }),
        "Prompt copied",
      );
      setCopied(success);
      if (success) onCopied?.();
      else
        setError(
          key
            ? "Could not copy the instructions. Your key is saved on this page; try copying again."
            : "Could not copy the instructions. Try copying again.",
        );
    } catch {
      setError(
        "Could not create a tracing key. Try again, or use a key already in your project.",
      );
    } finally {
      setCopying(false);
    }
  };
  const copyLabel = tracingKey
    ? "Copy instructions with key"
    : prepareTracingKey
      ? "Copy instructions with new key"
      : prominent
        ? "Copy setup instructions"
        : "Set it up for me";
  return (
    <div
      className={
        prominent
          ? "w-full min-w-0 text-sm"
          : "my-4 w-full rounded-lg border bg-muted/20 p-4 text-sm"
      }
    >
      {prominent && (
        <div className="mb-5 space-y-4">
          <div>
            <div className="flex flex-wrap items-center gap-3">
              <span className="inline-flex items-center gap-2 rounded-lg border bg-background px-3 py-2 text-sm font-medium">
                <img
                  src={claudeCodeLogo.src}
                  alt=""
                  className="size-7 object-contain"
                />
                Claude Code
              </span>
              <span className="inline-flex items-center gap-2 rounded-lg border bg-background px-3 py-2 text-sm font-medium">
                <img
                  src={codexLogo.src}
                  alt=""
                  className="size-6 object-contain dark:invert"
                />
                Codex
              </span>
              <span className="inline-flex items-center gap-2 rounded-lg border bg-background px-3 py-2 text-sm font-medium">
                <img
                  src={cursorLogo.src}
                  alt=""
                  className="size-6 object-contain dark:invert"
                />
                Cursor
              </span>
            </div>
            <p className="mt-2 text-sm leading-6 text-muted-foreground">
              Open your project in your coding agent, then paste these
              instructions.
            </p>
          </div>
          <p className="text-sm leading-6 text-muted-foreground">
            The instructions help your coding agent configure tracing, run one
            real task, and verify its trace arrived.
          </p>
        </div>
      )}
      <div className="flex flex-wrap items-center gap-3">
        <Button
          ref={copyButton}
          variant={prominent && primaryAction ? "default" : "outline"}
          size={prominent ? "default" : "sm"}
          aria-label={copyLabel}
          disabled={copying}
          onClick={() => void copy()}
        >
          {copying ? (
            <Loader2
              aria-hidden
              className="size-3.5 motion-safe:animate-spin"
            />
          ) : copied ? (
            <Check aria-hidden="true" className="size-3.5" />
          ) : (
            <Copy aria-hidden="true" className="size-3.5" />
          )}
          {copying
            ? "Preparing instructions…"
            : copied
              ? "Prompt copied"
              : copyLabel}
        </Button>
        <p className="text-xs leading-5 text-muted-foreground">
          {prominent
            ? tracingKey
              ? "Includes your tracing key. Paste only into a coding agent you trust; the key will be in its chat history."
              : prepareTracingKey
                ? "Creates a tracing key and includes it in the instructions. The key will be in your coding agent’s chat history."
                : "Includes your Lens address. Uses the tracing key in your project."
            : "Copy a prompt for your coding agent. It checks your setup and asks only for missing choices."}
        </p>
        {tracingKey && (
          <Button
            variant="link"
            size="sm"
            className="px-0"
            disabled={copying}
            onClick={() => void copy(false)}
          >
            Copy without key
          </Button>
        )}
      </div>
      {error && (
        <p role="alert" className="mt-3 text-sm text-destructive">
          {error}
        </p>
      )}
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
        {tracingKey && (
          <p className="mt-2 text-muted-foreground">
            The preview masks your key. Copy instructions with key includes its
            full value.
          </p>
        )}
        {!tracingKey && prepareTracingKey && (
          <p className="mt-2 text-muted-foreground">
            Lens adds the new tracing key when you copy these instructions.
          </p>
        )}
      </details>
    </div>
  );
}
