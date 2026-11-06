import { builtinEnvironments, type Environment } from "vitest/runtime";

const env: Environment = {
  name: "jsdom-fetch",
  viteEnvironment: "client",
  async setup(global, options) {
    const nativeAbortController = global.AbortController;
    const nativeAbortSignal = global.AbortSignal;
    const { teardown } = await builtinEnvironments.jsdom.setup(global, options);
    const needsAnimationEvent = !global.AnimationEvent;
    if (needsAnimationEvent) {
      // jsdom exposes prefixed CSS properties but no AnimationEvent; React otherwise selects WebKit events.
      global.AnimationEvent = class AnimationEvent extends global.Event {
        animationName: string;
        elapsedTime: number;
        pseudoElement: string;
        constructor(type: string, init: AnimationEventInit = {}) {
          super(type, init);
          this.animationName = init.animationName ?? "";
          this.elapsedTime = init.elapsedTime ?? 0;
          this.pseudoElement = init.pseudoElement ?? "";
        }
      };
    }
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
    return {
      teardown(environmentGlobal) {
        if (needsAnimationEvent)
          Reflect.deleteProperty(environmentGlobal, "AnimationEvent");
        return teardown(environmentGlobal);
      },
    };
  },
};

export default env;
