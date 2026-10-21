import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [react()],
  test: {
    // Keep unit tests scoped to product packages as the workspace grows.
    include: ["ui/src/**/*.test.{ts,tsx}"],
    environment: "jsdom",
    setupFiles: ["./ui/src/test/setup.ts"],
    clearMocks: true,
  },
});
