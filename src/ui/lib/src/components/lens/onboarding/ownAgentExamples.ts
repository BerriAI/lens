import type { SetupPath } from "./setupPrompt";

const PYTHON_OTEL = "opentelemetry-sdk opentelemetry-exporter-otlp-proto-http";
const NODE_OTEL =
  "@opentelemetry/sdk-trace-node @opentelemetry/exporter-trace-otlp-proto@0.219";

const python = (imports: string, setup: string, run: string) => `import json
import os
from opentelemetry.exporter.otlp.proto.http.trace_exporter import OTLPSpanExporter
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import SimpleSpanProcessor
${imports}

provider = TracerProvider()
provider.add_span_processor(SimpleSpanProcessor(OTLPSpanExporter()))
tracer = provider.get_tracer("my-agent")
prompt = "Reply with the word hello."
model = os.environ["OPENAI_MODEL"]
${setup}

def message(role, text):
    return json.dumps([{"role": role, "parts": [{"type": "text", "content": text}]}])

input_messages = message("user", prompt)
try:
    with tracer.start_as_current_span("invoke_agent my-agent", attributes={
        "gen_ai.operation.name": "invoke_agent",
        "gen_ai.agent.name": "my-agent",
    }):
        with tracer.start_as_current_span("chat", attributes={
            "gen_ai.operation.name": "chat",
            "gen_ai.request.model": model,
            "gen_ai.input.messages": input_messages,
        }) as call:
${run.replaceAll(/^/gm, "    ")}
            call.set_attribute("gen_ai.output.messages", message("assistant", reply))
        print(reply)
finally:
    provider.shutdown()
`;

const node = (imports: string, run: string) => `import { NodeTracerProvider, SimpleSpanProcessor } from "@opentelemetry/sdk-trace-node";
import { OTLPTraceExporter } from "@opentelemetry/exporter-trace-otlp-proto";
${imports}

const provider = new NodeTracerProvider({
  spanProcessors: [new SimpleSpanProcessor(new OTLPTraceExporter())],
});
provider.register();
const tracer = provider.getTracer("my-agent");
const prompt = "Reply with the word hello.";
const model = process.env.OPENAI_MODEL;
if (!model) throw new Error("Set OPENAI_MODEL to a model you can use");
const message = (role: string, content: string) =>
  JSON.stringify([{ role, parts: [{ type: "text", content }] }]);

try {
  await tracer.startActiveSpan("invoke_agent my-agent", {
    attributes: {
      "gen_ai.operation.name": "invoke_agent",
      "gen_ai.agent.name": "my-agent",
    },
  }, async (span) => {
    try {
      await tracer.startActiveSpan("chat", {
        attributes: {
          "gen_ai.operation.name": "chat",
          "gen_ai.request.model": model,
          "gen_ai.input.messages": message("user", prompt),
        },
      }, async (call) => {
        try {
${run.replaceAll(/^/gm, "    ")}
          call.setAttribute("gen_ai.output.messages", message("assistant", reply));
          console.log(reply);
        } finally {
          call.end();
        }
      });
    } finally {
      span.end();
    }
  });
} finally {
  await provider.shutdown();
}
`;

export const OWN_AGENT_FRAMEWORKS = [
  {
    id: "otel-python",
    label: "OpenTelemetry (Python)",
    file: "agent.py",
    install: `pip install ${PYTHON_OTEL} openai`,
    run: "python agent.py",
    code: python(
      "from openai import OpenAI",
      "client = OpenAI()",
      "        reply = client.responses.create(model=model, input=prompt).output_text",
    ),
  },
  {
    id: "otel-typescript",
    label: "OpenTelemetry (TypeScript)",
    file: "agent.mts",
    install: `npm install ${NODE_OTEL} openai tsx`,
    run: "npx tsx agent.mts",
    code: node(
      'import OpenAI from "openai";',
      `      const client = new OpenAI();
      const reply = (await client.responses.create({ model, input: prompt })).output_text;`,
    ),
  },
  {
    id: "openai-agents",
    label: "OpenAI Agents SDK",
    file: "agent.py",
    install: `pip install ${PYTHON_OTEL} openai-agents`,
    run: "python agent.py",
    code: python(
      "from agents import Agent, Runner, set_tracing_disabled",
      'set_tracing_disabled(True)\nagent = Agent(name="my-agent", model=model)',
      "        reply = Runner.run_sync(agent, prompt).final_output",
    ),
  },
  {
    id: "vercel",
    label: "Vercel AI SDK",
    file: "agent.mts",
    install: `npm install ${NODE_OTEL} ai @ai-sdk/openai tsx`,
    run: "npx tsx agent.mts",
    code: node(
      'import { generateText } from "ai";\nimport { openai } from "@ai-sdk/openai";',
      "      const { text: reply } = await generateText({ model: openai(model), prompt });",
    ),
  },
  {
    id: "langgraph",
    label: "LangGraph",
    file: "agent.py",
    install: `pip install ${PYTHON_OTEL} langgraph openai`,
    run: "python agent.py",
    code: python(
      "from langgraph.graph import END, START, MessagesState, StateGraph\nfrom openai import OpenAI",
      `client = OpenAI()

def respond(state: MessagesState):
    request = state["messages"][-1].content
    reply = client.responses.create(model=model, input=request).output_text
    return {"messages": [{"role": "assistant", "content": reply}]}

graph = StateGraph(MessagesState).add_node("respond", respond)
agent = graph.add_edge(START, "respond").add_edge("respond", END).compile()`,
      `        result = agent.invoke({"messages": [{"role": "user", "content": prompt}]})
        reply = result["messages"][-1].content`,
    ),
  },
] as const;

const shellValue = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;

export function ownAgentEnvironment(
  path: SetupPath,
  baseUrl: string,
  tracingKey?: string,
) {
  const gateway = path === "gateway";
  const variable = gateway ? "LITELLM_API_KEY" : "LITELLM_TRACING_KEY";
  const key = gateway ? "<your LiteLLM virtual key>" : tracingKey ?? "<your tracing key>";
  const base = baseUrl.replace(/\/+$/, "") ||
    (gateway ? "<your LiteLLM gateway address>" : "<your Lens address>");
  return [
    `export ${variable}=${shellValue(key)}`,
    `export OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=${shellValue(`${base}/v1/traces`)}`,
    `export OTEL_EXPORTER_OTLP_TRACES_HEADERS="Authorization=Bearer%20$${variable}"`,
    "export OPENAI_API_KEY='<your OpenAI provider key>'",
    "export OPENAI_MODEL='<your OpenAI model>'",
  ].join("\n");
}
