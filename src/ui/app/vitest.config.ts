import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "jsdom",
    include: ["app/**/*.test.tsx"],
    setupFiles: ["tests/setup.ts"],
    clearMocks: true,
  },
  esbuild: { jsx: "automatic", jsxImportSource: "react" },
});

