import { createServer, request as forward } from "node:http";
import { existsSync, mkdirSync, readFileSync, writeFileSync, createWriteStream } from "node:fs";
import { spawn, execFileSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const gateway = process.env.LENS_GATEWAY_SOURCE;
if (!gateway || !existsSync(resolve(gateway, "ui/litellm-dashboard/package.json"))) {
  console.error("Set LENS_GATEWAY_SOURCE to a LiteLLM checkout with the embedded Lens dashboard.");
  process.exit(1);
}
const python = process.env.LENS_GATEWAY_PYTHON ?? resolve(gateway, ".venv/bin/python");
if (!existsSync(python)) {
  console.error("Set LENS_GATEWAY_PYTHON to a Python environment with the gateway's proxy dependencies installed.");
  process.exit(1);
}
const port = Number(process.env.LENS_DEMO_PORT ?? 3100);
if (!Number.isInteger(port) || port < 1024 || port > 11977) {
  console.error("LENS_DEMO_PORT must be an integer between 1024 and 11977.");
  process.exit(1);
}
const backendPort = port + 1100;
const dashboardPort = port + 1;
const databasePort = port + 53558;
const runtime = resolve(root, ".lens-dev/gateway-demo");
const dashboard = resolve(gateway, "ui/litellm-dashboard");
const backendSource = process.env.LENS_GATEWAY_BACKEND_SOURCE ?? gateway;
const origin = `http://127.0.0.1:${port}`;
const backend = `http://127.0.0.1:${backendPort}`;
const destination = "/lens?demo=true&tab=optimizations";
mkdirSync(runtime, { recursive: true });
const baseEnv = Object.fromEntries(
  ["PATH", "HOME", "TMPDIR", "LANG", "SYSTEMROOT"]
    .filter((key) => process.env[key])
    .map((key) => [key, process.env[key]]),
);
const container = `lens-optimization-demo-postgres-${port}`;
try {
  execFileSync("docker", ["inspect", container], { stdio: "ignore" });
  execFileSync("docker", ["start", container], { stdio: "ignore" });
} catch {
  execFileSync(
    "docker",
    [
      "run",
      "--detach",
      "--name",
      container,
      "--publish",
      `127.0.0.1:${databasePort}:5432`,
      "--env",
      "POSTGRES_HOST_AUTH_METHOD=trust",
      "--env",
      "POSTGRES_DB=lens_demo",
      "postgres:17",
    ],
    { stdio: "ignore" },
  );
}
console.log("Preparing the Lens package in the actual gateway dashboard…");
const packed = JSON.parse(
  execFileSync("npm", ["pack", "--workspace", "@litellm/lens-ui", "--pack-destination", runtime, "--json"], {
    cwd: root,
    encoding: "utf8",
  }),
)[0];
const installed = resolve(dashboard, "node_modules/@litellm/lens-ui");
if (
  existsSync(resolve(installed, "package.json")) &&
  JSON.parse(readFileSync(resolve(installed, "package.json"), "utf8")).version === packed.version
) {
  execFileSync("tar", ["-xzf", resolve(runtime, packed.filename), "--strip-components=1", "-C", installed]);
} else {
  execFileSync(
    "npm",
    [
      "install",
      "--no-save",
      "--package-lock=false",
      "--ignore-scripts",
      "--no-audit",
      "--no-fund",
      resolve(runtime, packed.filename),
    ],
    { cwd: dashboard, stdio: "inherit" },
  );
}
const password = randomBytes(32).toString("hex");
const masterKey = `sk-${randomBytes(32).toString("hex")}`;
const databaseUrl = `postgresql://postgres@127.0.0.1:${databasePort}/lens_demo`;
const configPath = resolve(runtime, "gateway.yaml");
writeFileSync(
  configPath,
  "model_list: []\ngeneral_settings:\n  master_key: os.environ/LITELLM_MASTER_KEY\nlitellm_settings:\n  telemetry: false\n",
);
const log = createWriteStream(resolve(runtime, "gateway.log"), { flags: "w" });
const redact = (text) =>
  [password, masterKey, databaseUrl].reduce((value, secret) => value.replaceAll(secret, "[redacted]"), text);
const proxy = spawn(
  python,
  [
    resolve(backendSource, "litellm/proxy/proxy_cli.py"),
    "--config",
    configPath,
    "--host",
    "127.0.0.1",
    "--port",
    String(backendPort),
    "--use_v2_migration_resolver",
  ],
  {
    cwd: backendSource,
    env: {
      ...baseEnv,
      PYTHONPATH: backendSource,
      PYTHON_DOTENV_DISABLED: "1",
      DATABASE_URL: databaseUrl,
      LITELLM_MASTER_KEY: masterKey,
      UI_USERNAME: "demo",
      UI_PASSWORD: password,
      LITELLM_LOCAL_MODEL_COST_MAP: "True",
      LITELLM_UI_PATH: resolve(runtime, "ui"),
      DISABLE_ADMIN_UI: "False",
      LITELLM_TELEMETRY: "False",
    },
    stdio: ["ignore", "pipe", "pipe"],
  },
);
proxy.stdout.on("data", (chunk) => log.write(redact(chunk.toString())));
proxy.stderr.on("data", (chunk) => log.write(redact(chunk.toString())));
const web = spawn("npm", ["run", "dev", "--", "--hostname", "127.0.0.1", "--port", String(dashboardPort)], {
  cwd: dashboard,
  env: {
    ...baseEnv,
    LENS_DEV_PROXY_URL: backend,
    NEXT_PUBLIC_USE_REWRITES: "true",
    NEXT_TELEMETRY_DISABLED: "1",
  },
  stdio: ["ignore", "pipe", "pipe"],
});
web.stdout.on("data", (chunk) => log.write(redact(chunk.toString())));
web.stderr.on("data", (chunk) => log.write(redact(chunk.toString())));
let stopping = false;
const stop = () => {
  if (stopping) return;
  stopping = true;
  proxy.kill("SIGTERM");
  web.kill("SIGTERM");
  server.close();
};
for (const child of [proxy, web])
  child.on("exit", (code) => {
    if (!stopping) {
      console.error(`A demo process stopped (${code}). See .lens-dev/gateway-demo/gateway.log`);
      stop();
      process.exitCode = 1;
    }
  });
process.on("SIGINT", stop);
process.on("SIGTERM", stop);
const server = createServer(async (request, response) => {
  if (request.headers.host !== `127.0.0.1:${port}` && request.headers.host !== `localhost:${port}`) {
    response.writeHead(403).end();
    return;
  }
  if (request.url === "/demo-health") {
    response.writeHead(200, { "Content-Type": "text/plain" }).end("Lens gateway demo");
    return;
  }
  if (request.url === "/" || request.url === "/start" || request.url?.startsWith("/ui/demo/")) {
    try {
      const login = await fetch(`${backend}/v2/login`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ username: "demo", password }),
      });
      if (!login.ok) {
        response
          .writeHead(503, { "Content-Type": "text/plain", Refresh: "3" })
          .end("The local gateway is starting. This page will retry shortly.");
        return;
      }
      response
        .writeHead(302, {
          "Set-Cookie": login.headers.getSetCookie(),
          Location: destination,
        })
        .end();
    } catch {
      response
        .writeHead(503, { "Content-Type": "text/plain", Refresh: "3" })
        .end("The local gateway is starting. This page will retry shortly.");
    }
    return;
  }
  const upstream = forward(
    {
      hostname: "127.0.0.1",
      port: dashboardPort,
      path: request.url,
      method: request.method,
      headers: request.headers,
    },
    (incoming) => {
      response.writeHead(incoming.statusCode ?? 502, incoming.headers);
      incoming.pipe(response);
    },
  );
  upstream.on("error", () => {
    if (!response.headersSent) response.writeHead(503, { "Content-Type": "text/plain", Refresh: "3" });
    response.end("The gateway dashboard is starting. This page will retry shortly.");
  });
  request.pipe(upstream);
});
server.on("upgrade", (request, socket, head) => {
  const upstream = forward({
    hostname: "127.0.0.1",
    port: dashboardPort,
    path: request.url,
    headers: request.headers,
  });
  upstream.on("upgrade", (response, remote, remoteHead) => {
    socket.write(
      `HTTP/1.1 101 Switching Protocols\r\n${Object.entries(response.headers)
        .map(([key, value]) => `${key}: ${value}\r\n`)
        .join("")}\r\n`,
    );
    remote.write(head);
    socket.write(remoteHead);
    remote.pipe(socket);
    socket.pipe(remote);
  });
  upstream.on("error", () => socket.destroy());
  upstream.end();
});
server.on("error", (error) => {
  console.error(error.code === "EADDRINUSE" ? `Port ${port} is already in use.` : error.message);
  stop();
});
server.listen(port, "127.0.0.1", () => {
  console.log(
    `Open ${origin}/\nActual LiteLLM gateway and embedded Lens. Lens traffic and results use demo data.\nKeep this terminal open. Press Ctrl+C to stop.`,
  );
  if (process.env.LENS_DEMO_OPEN === "1" && process.platform === "darwin") {
    spawn("open", [`${origin}/`], { stdio: "ignore" });
  }
});
