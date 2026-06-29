import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test-setup.ts"],
  },
  esbuild: {
    tsconfigRaw: {
      compilerOptions: {
        jsx: "react-jsx",
        types: ["vitest/globals", "@testing-library/jest-dom"],
      },
    },
  },
});
