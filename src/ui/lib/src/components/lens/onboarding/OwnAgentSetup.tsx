import { useState } from "react";
import { Button } from "../../ui/button";
import { ownAgentEnvironment, OWN_AGENT_FRAMEWORKS } from "./ownAgentExamples";
import type { SetupPath } from "./setupPrompt";
import { CodeBlock } from "./tracing/TracingSetupCard";
import { maskSecret } from "./tracing/tracingKeys";

export function OwnAgentSetup({
  path,
  baseUrl,
  tracingKey,
  onCopied,
}: {
  readonly path: SetupPath;
  readonly baseUrl: string;
  readonly tracingKey?: string;
  readonly onCopied?: () => void;
}) {
  const [example, setExample] = useState<(typeof OWN_AGENT_FRAMEWORKS)[number]>(
    OWN_AGENT_FRAMEWORKS[0],
  );
  return (
    <div className="space-y-3">
      <p className="text-sm leading-6 text-muted-foreground">
        Instrument an agent you build with OpenTelemetry. These examples send the
        prompt and reply to Lens. Only capture content you are allowed to send.
        Use a separate provider key for the example&apos;s model calls.
      </p>
      <div role="radiogroup" aria-label="Framework" className="flex flex-wrap gap-2">
        {OWN_AGENT_FRAMEWORKS.map((option) => (
          <Button
            key={option.id}
            role="radio"
            aria-checked={example.id === option.id}
            size="sm"
            variant={example.id === option.id ? "default" : "outline"}
            onClick={() => setExample(option)}
          >
            {option.label}
          </Button>
        ))}
      </div>
      <p className="text-sm font-medium">Configure the environment and install dependencies</p>
      <CodeBlock
        code={`${ownAgentEnvironment(path, baseUrl, tracingKey)}\n${example.install}`}
        display={`${ownAgentEnvironment(path, baseUrl, tracingKey ? maskSecret(tracingKey) : undefined)}\n${example.install}`}
        copyLabel="Copy own-agent environment and install command"
        wrap
        onCopied={onCopied}
      />
      <p className="text-sm font-medium">Save the example as <code>{example.file}</code></p>
      <CodeBlock code={example.code} copyLabel={`Copy ${example.label} example`} />
      <p className="text-sm text-muted-foreground">
        Run <code>{example.run}</code> and look for <code>my-agent</code> in Agents.
        Change <code>gen_ai.agent.name</code> to your agent&apos;s name.
      </p>
    </div>
  );
}
