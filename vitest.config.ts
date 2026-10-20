import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [react()],
  test: {
    include: ["ui/src/**/*.test.{ts,tsx}", "plugins/**/*.test.ts"],
    environment: "jsdom",
    setupFiles: ["./ui/src/test/setup.ts"],
    clearMocks: true,
  },
});
