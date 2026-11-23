"use client";

import { useEffect, useState } from "react";
import { ArrowUpRight, Plug } from "lucide-react";
import { useLensHost } from "../../../host/LensHost";
import { hostedApiRoot } from "../../../lib/http/resolveApiBase";
import { getRequestBaseUrl } from "../../../lib/http/runtime";
import { CodeBlock } from "../onboarding/tracing/TracingSetupCard";
import { MCP_DOCS_URL, mcpConnectionGuide } from "./mcpConnection";
import { SettingsCard, SettingsSection } from "./SettingsSection";

export function McpConnection() {
  const embedded = useLensHost().surface !== "standalone";
  const [guide, setGuide] = useState<ReturnType<typeof mcpConnectionGuide> | null>(null);
  useEffect(() => {
    const base = getRequestBaseUrl();
    setGuide(
      mcpConnectionGuide(
        /^https?:\/\//.test(base) ? base : `${window.location.origin}${base}`,
        embedded ? null : hostedApiRoot(window.location.pathname),
        embedded,
      ),
    );
  }, [embedded]);

  return (
    <SettingsSection
      heading="MCP server"
      icon={<Plug aria-hidden="true" className="size-4" />}
      description="Let your coding agent read its own traces from Lens."
    >
      <SettingsCard className="space-y-3">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <p className="text-sm font-medium">Read only access to traces, steps and cost</p>
          <a
            href={MCP_DOCS_URL}
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex items-center gap-1 text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
          >
            Docs
            <ArrowUpRight aria-hidden="true" className="size-3" />
          </a>
        </div>
        {guide && (
          <>
            <p className="break-all font-mono text-xs text-muted-foreground">{guide.url}</p>
            <p className="text-xs text-muted-foreground">{guide.credential}</p>
            <h3 className="text-xs font-medium">Claude Code</h3>
            <CodeBlock code={guide.claudeCode} copyLabel="Copy Claude Code command" />
            <h3 className="text-xs font-medium">Codex</h3>
            <CodeBlock code={guide.codex} copyLabel="Copy Codex command" />
          </>
        )}
      </SettingsCard>
    </SettingsSection>
  );
}
