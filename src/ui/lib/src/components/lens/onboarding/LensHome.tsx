"use client";

import { ArrowRight, Check, Plus } from "lucide-react";
import { Button } from "../../ui/button";
import type { LensAgents } from "../agents/AgentScoped";
import { cn } from "../../../lib/cva.config";

export function LensHome({
  agents,
  onAddAgent,
  onOpenAgents,
  onSetup,
}: {
  readonly agents: LensAgents;
  readonly onAddAgent?: () => void;
  readonly onOpenAgents: () => void;
  readonly onSetup?: () => void;
}) {
  const connected = agents.list.agents.length > 0;
  return (
    <section aria-label="Get started with Lens" className="mx-auto w-full max-w-4xl px-3 py-10 sm:px-6 sm:py-14">
      <div className="mb-7 space-y-2">
        <h2 className="text-2xl font-semibold tracking-tight">
          {connected ? "Your workspace" : "Get started with Lens"}
        </h2>
        <p className="text-sm text-muted-foreground">
          {connected
            ? "Explore your agents or connect another project."
            : "Connect your agent and see its first trace."}
        </p>
      </div>
      <div className="overflow-hidden rounded-xl border bg-card">
        <div className="flex flex-wrap items-center justify-between gap-4 border-b p-5 sm:p-6">
          <div className="space-y-1">
            <h3 className="font-medium">{connected ? "Agents connected" : "Connect your first agent"}</h3>
            <p className="text-sm text-muted-foreground">
              {connected
                ? `${agents.list.agents.length} ${agents.list.agents.length === 1 ? "agent has" : "agents have"} sent traces in the last 14 days`
                : "Add an agent, point its traces to Lens, then run a task."}
            </p>
          </div>
          {onAddAgent && (
            <Button size="sm" onClick={onAddAgent}>
              <Plus aria-hidden className="size-4" />
              Add agent
            </Button>
          )}
        </div>
        <ol className="divide-y px-5 sm:px-6">
          {[
            { title: "Add your agent", detail: "Choose a name and how your project sends traces." },
            { title: "Connect tracing", detail: "Copy your Lens endpoint and tracing key into your project." },
            { title: "Run a task", detail: "Your agent appears in Lens when its first trace arrives." },
          ].map(({ title, detail }, index) => (
            <li key={title} className="flex gap-4 py-5">
              <span
                className={cn(
                  "flex size-7 shrink-0 items-center justify-center rounded-full border text-xs font-medium",
                  connected ? "border-success/20 bg-success/10 text-success" : "bg-muted/50 text-muted-foreground",
                )}
              >
                {connected ? <Check aria-hidden className="size-3.5" /> : index + 1}
              </span>
              <div className="space-y-1">
                <p className="text-sm font-medium">{title}</p>
                <p className="text-sm text-muted-foreground">{detail}</p>
              </div>
            </li>
          ))}
        </ol>
        {connected && (
          <div className="border-t px-5 py-4 sm:px-6">
            <Button variant="outline" size="sm" onClick={onOpenAgents}>
              View agents
              <ArrowRight aria-hidden className="size-3.5" />
            </Button>
          </div>
        )}
      </div>
      {!onAddAgent && !connected && (
        <p className="mt-4 text-sm text-muted-foreground">
          Ask your Lens administrator for a tracing key to connect your agent.
        </p>
      )}
      {agents.list.error && (
        <p role="alert" className="mt-4 text-sm text-destructive">
          Could not check connected agents. Try opening Agents again.
        </p>
      )}
      {onSetup && (
        <Button variant="link" size="sm" className="mt-4 px-0 text-xs text-muted-foreground" onClick={onSetup}>
          Deployment setup
        </Button>
      )}
    </section>
  );
}
