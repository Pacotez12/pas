import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["e2e/**/*.spec.ts"],
    fileParallelism: false,
    maxConcurrency: 1,
    testTimeout: 60_000,
    hookTimeout: 30_000,
  },
});
