import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  // Bind only to loopback because the development UI has no network role.
  server: { host: "127.0.0.1", port: 1420, strictPort: true },
  build: { target: "safari16" },
});
