// Flat ESLint config (ESLint 9). Encodes the frontend boundary rules (07 §6):
//  • no `any` across the IPC boundary,
//  • `@tauri-apps/api` may be imported ONLY inside `src/ipc/`,
//  • presentational components use i18n keys, not literal strings (T008).
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import i18next from "eslint-plugin-i18next";

export default tseslint.config(
  {
    ignores: [
      "dist/**",
      "coverage/**",
      "node_modules/**",
      "src-tauri/**",
      // Generated — do not lint/format (T003).
      "packages/shared/src/bindings.ts",
    ],
  },

  ...tseslint.configs.recommended,

  {
    files: ["src/**/*.{ts,tsx}", "packages/**/*.ts"],
    plugins: {
      "react-hooks": reactHooks,
      "react-refresh": reactRefresh,
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      "react-refresh/only-export-components": ["warn", { allowConstantExport: true }],
      "@typescript-eslint/no-explicit-any": "error",
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
      ],
      // The IPC boundary: only src/ipc may reach @tauri-apps/api (re-enabled below).
      "no-restricted-imports": [
        "error",
        {
          paths: [{ name: "@tauri-apps/api", message: "Import IPC through src/ipc only (07 §6)." }],
          patterns: [
            { group: ["@tauri-apps/api/*"], message: "Import IPC through src/ipc only (07 §6)." },
          ],
        },
      ],
    },
  },

  // The single data-access layer is allowed to import @tauri-apps/api.
  {
    files: ["src/ipc/**/*.ts"],
    rules: { "no-restricted-imports": "off" },
  },

  // Presentational components must render i18n keys, never literal display text.
  {
    files: ["src/components/**/*.tsx", "src/routes/**/*.tsx"],
    plugins: { i18next },
    rules: {
      "i18next/no-literal-string": ["warn", { mode: "jsx-text-only" }],
    },
  },

  // Test files may use any-ish fixtures and literal strings freely.
  {
    files: ["**/*.test.{ts,tsx}", "vitest.setup.ts"],
    plugins: { i18next },
    rules: {
      "@typescript-eslint/no-explicit-any": "off",
      "i18next/no-literal-string": "off",
    },
  },
);
