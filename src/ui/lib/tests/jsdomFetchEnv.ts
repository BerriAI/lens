import { builtinEnvironments, type Environment } from "vitest/runtime";

const env: Environment = {
  name: "jsdom-fetch",
  viteEnvironment: "client",
  async setup(global, options) {
    const nativeAbortController = global.AbortController;
    const nativeAbortSignal = global.AbortSignal;
    const { teardown } = await builtinEnvironments.jsdom.setup(global, options);
    Object.defineProperty(global, "AbortController", {
      configurable: true,
      writable: true,
      value: nativeAbortController,
    });
    Object.defineProperty(global, "AbortSignal", {
      configurable: true,
      writable: true,
      value: nativeAbortSignal,
    });
    return { teardown };
  },
};

export default env;
