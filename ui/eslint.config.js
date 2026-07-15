// ESLint v9 flat config for the Talon UI.
//
// We lint TypeScript + TSX. The recommended preset from @eslint/js
// catches common mistakes without being noisy. typescript-eslint's
// "stylistic" rules are off by default; we add a few for code-quality
// signals (unused vars, no-floating-promises) that matter in a Tauri
// IPC app where unhandled promise rejections are a real bug class.
//
// Scope: src/ only. We do not lint build artifacts (dist/), node_modules,
// or generated files. The config file itself is excluded to avoid
// bootstrap loops.
import js from "@eslint/js";
import tseslint from "typescript-eslint";

export default tseslint.config(
  {
    ignores: ["dist/**", "node_modules/**", "*.config.js", "*.config.ts"],
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["src/**/*.{ts,tsx}"],
    languageOptions: {
      ecmaVersion: 2022,
      sourceType: "module",
      globals: {
        // Browser globals the React app uses.
        window: "readonly",
        document: "readonly",
        console: "readonly",
        HTMLElement: "readonly",
        // Vitest/Testing Library globals (set by vitest.config.ts).
        describe: "readonly",
        it: "readonly",
        test: "readonly",
        expect: "readonly",
        beforeEach: "readonly",
        afterEach: "readonly",
        beforeAll: "readonly",
        afterAll: "readonly",
        vi: "readonly",
      },
    },
    rules: {
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
      ],
      "@typescript-eslint/no-explicit-any": "warn",
      // no-floating-promises is intentionally OFF: it requires type-aware
      // linting (a TS project reference wired into parserOptions), which
      // is heavier than a phase-1 skeleton needs. Add it back when the
      // IPC contract grows and unhandled rejections become a real risk.
    },
  },
);
