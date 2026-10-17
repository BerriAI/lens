import OpenAI from "openai";
import { Agent, fetch as undiciFetch } from "undici";

const dispatcher = new Agent({ headersTimeout: 0, bodyTimeout: 0 });

export class InvestigationClient extends OpenAI {
  private readonly requestFetch: typeof fetch;

  constructor(options: ConstructorParameters<typeof OpenAI>[0]) {
    super({ ...options, timeout: 0, fetchOptions: { dispatcher } });
    this.requestFetch =
      options?.fetch ?? (undiciFetch as unknown as typeof fetch);
  }

  override async fetchWithTimeout(
    url: RequestInfo,
    init: RequestInit | undefined,
    _milliseconds: number,
    controller: AbortController,
  ): Promise<Response> {
    return this.requestFetch(url, {
      ...init,
      signal: init?.signal
        ? AbortSignal.any([init.signal, controller.signal])
        : controller.signal,
    });
  }
}
