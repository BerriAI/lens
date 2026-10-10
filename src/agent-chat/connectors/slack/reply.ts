import { responder, type Respond } from "@litellm/lens-agent/agent";
import type { Turn, Answer } from "@litellm/lens-agent/models";
import { agentConfig, type Config } from "./config.js";
import type { FindingThread } from "./threads.js";
import { safeAnswer } from "./slack.js";
export type ReplyAgent = (
  history: readonly Turn[],
  signal: AbortSignal,
  finding?: FindingThread,
) => Promise<Answer>;
export function replyAgent(
  config: Config,
  respond: Respond = responder(agentConfig(config)),
): ReplyAgent {
  return async (history, signal, finding) => {
    const result = await respond(history, signal, finding);
    return safeAnswer(
      result.answer,
      new Set(result.evidenceUrls),
      result.detailed,
      result.frequencies,
      finding ? 4 : 2,
    );
  };
}
