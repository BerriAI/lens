import { useState } from "react";
import { Button } from "../../ui/button";
import type { Optimization } from "./model";

export function RunnerSetup({ run }: { run: Optimization }) {
  const [copied, setCopied] = useState(false);
  const live = run.config.mode === "live";
  const initialization = `import os\nfrom lens import Lens, Run\nfrom my_agent import agent\n\nlens = Lens(os.environ["LENS_BASE_URL"], os.environ["LENS_API_KEY"])\n`;
  const snippet = live
    ? `${initialization}\nassignment = lens.optimizations.assign(\n    agent=${JSON.stringify(run.config.agent)},\n    baseline=${JSON.stringify(run.config.baseline)},\n    trace_id=trace_id,\n)\nresult = await agent.run(input, model=assignment.model, trace_id=trace_id)\n`
    : `${initialization}\nasync def task(case, model):\n    result = await agent.run(\n        case.input,\n        model=model,\n        trace_attributes={"lens.optimization.request_id": case.meta["request_id"]},\n    )\n    return Run(trace={"trace_id": result.trace_id})\n\nawait lens.optimizations.arun(${JSON.stringify(run.id)}, task=task)\n`;
  return <details className="rounded-lg border p-4" open={run.completed === 0}>
    <summary className="cursor-pointer text-sm font-medium">{live ? "Connect live routing" : "Run your agent"}</summary>
    <p className="mt-3 text-xs text-muted-foreground">{live ? "Adapt this to your traced agent. Select the model once per run." : "Adapt this to your agent. Record the trial ID on its root span and return the completed trace."}</p>
    <pre className="mt-3 overflow-auto rounded-md bg-muted/40 p-3 text-xs leading-5">{snippet}</pre>
    <Button className="mt-3" size="xs" variant="outline" onClick={() => { void navigator.clipboard.writeText(snippet).then(() => setCopied(true)); }}>{copied ? "Copied" : "Copy snippet"}</Button>
  </details>;
}
