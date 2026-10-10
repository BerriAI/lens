import { spawn, type ChildProcess } from "node:child_process";
import { pathToFileURL } from "node:url";

export interface Command {
  readonly file: string;
  readonly args: readonly string[];
}

export async function supervise(
  server: Command,
  sidecar?: Command,
  restartMs = 30_000,
): Promise<number> {
  const child = spawn(server.file, [...server.args], { stdio: "inherit" });
  let helper: ChildProcess | undefined;
  let restart: NodeJS.Timeout | undefined;
  let stopping = false;
  let restarts = 0;
  const stop = () => {
    stopping = true;
    clearTimeout(restart);
    child.kill("SIGTERM");
    helper?.kill("SIGTERM");
  };
  const force = () => {
    child.kill("SIGKILL");
    helper?.kill("SIGKILL");
  };
  const signal = () => {
    stop();
    setTimeout(force, 12_000).unref();
  };
  process.once("SIGINT", signal);
  process.once("SIGTERM", signal);
  const launch = () => {
    if (!sidecar || stopping) return;
    helper = spawn(sidecar.file, [...sidecar.args], { stdio: "inherit" });
    helper.on("error", () => {
      console.error("Slack sidecar process failed");
    });
    helper.once("close", () => {
      if (stopping || restarts >= 3) return;
      restarts++;
      console.warn("Slack sidecar stopped; retrying startup");
      restart = setTimeout(launch, restartMs * restarts);
    });
  };
  launch();
  return new Promise((resolve) => {
    child.on("error", () => {
      console.error("Lens server process failed");
    });
    child.once("close", (code, exitSignal) => {
      stop();
      process.removeListener("SIGINT", signal);
      process.removeListener("SIGTERM", signal);
      const result = code ?? (exitSignal === "SIGTERM" ? 143 : 1);
      if (!helper || helper.exitCode !== null || helper.signalCode !== null) {
        resolve(result);
        return;
      }
      const timeout = setTimeout(() => {
        force();
        resolve(result);
      }, 12_000);
      helper.once("close", () => {
        clearTimeout(timeout);
        resolve(result);
      });
    });
  });
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  const args = process.argv.slice(2);
  const sidecar =
    args.length === 0 && process.env.LENS_SLACK_CHAT_ENABLED === "true"
      ? {
          file: process.execPath,
          args: [
            new URL("./connectors/slack/main.js", import.meta.url).pathname,
          ],
        }
      : undefined;
  process.exitCode = await supervise(
    { file: "/usr/local/bin/litellm-lens", args },
    sidecar,
  );
}
