import { realpathSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { defineConfig } from "oxlint";
import { configFromProject } from "@o-feod/oxlint-structure-plugin/configs";

const rootDir = realpathSync(fileURLToPath(new URL(".", import.meta.url)));
const feod = configFromProject({ rootDir, preset: "recommended" });

export default defineConfig({
  plugins: ["typescript", "react", "jsx-a11y", "import", "oxc"],
  ignorePatterns: [
    "target/**",
    "node_modules/**",
    "web/dist/**",
    "web/.generated/**",
    "web/.impeccable/**",
    ".agents/**",
    "skills/**",
  ],
  rules: {
    "react/button-has-type": "error",
    "react/exhaustive-deps": "error",
    "react/jsx-key": "error",
    "react/jsx-no-duplicate-props": "error",
    "react/jsx-no-target-blank": "error",
    "react/no-children-prop": "error",
    "react/no-danger": "error",
    "react/no-danger-with-children": "error",
    "react/no-direct-mutation-state": "error",
    "react/rules-of-hooks": "error",
    "jsx-a11y/alt-text": "error",
    "jsx-a11y/anchor-has-content": "error",
    "jsx-a11y/anchor-is-valid": "error",
    "jsx-a11y/aria-props": "error",
    "jsx-a11y/aria-proptypes": "error",
    "jsx-a11y/aria-role": "error",
    "jsx-a11y/aria-unsupported-elements": "error",
    "jsx-a11y/click-events-have-key-events": "error",
    "jsx-a11y/control-has-associated-label": "error",
    "jsx-a11y/heading-has-content": "error",
    "jsx-a11y/interactive-supports-focus": "error",
    "jsx-a11y/label-has-associated-control": "error",
    "jsx-a11y/mouse-events-have-key-events": "error",
    "jsx-a11y/no-access-key": "error",
    "jsx-a11y/no-aria-hidden-on-focusable": "error",
    "jsx-a11y/no-autofocus": "error",
    "jsx-a11y/no-interactive-element-to-noninteractive-role": "error",
    "jsx-a11y/no-noninteractive-element-interactions": "error",
    "jsx-a11y/no-noninteractive-element-to-interactive-role": "error",
    "jsx-a11y/no-noninteractive-tabindex": "error",
    "jsx-a11y/no-redundant-roles": "error",
    "jsx-a11y/no-static-element-interactions": "error",
    "jsx-a11y/prefer-tag-over-role": "error",
    "jsx-a11y/role-has-required-aria-props": "error",
    "jsx-a11y/role-supports-aria-props": "error",
    "jsx-a11y/tabindex-no-positive": "error",
  },
  overrides: [
    {
      files: ["web/src/**/*.{ts,tsx,js,jsx}"],
      jsPlugins: feod.jsPlugins,
      rules: {
        ...feod.rules,
      },
    },
  ],
});
