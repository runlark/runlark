import "@mantine/core/styles/global.css";
import "@mantine/core/styles/Badge.css";
import "@mantine/core/styles/Button.css";
import "@mantine/core/styles/Input.css";
import "../workbench.css";
import "../board.css";

import { MantineProvider, createTheme } from "@mantine/core";
import { useState, type ReactNode } from "react";

const workbenchTheme = createTheme({
  fontFamily: '"Manrope", system-ui, sans-serif',
  fontFamilyMonospace: '"Roboto Mono", ui-monospace, monospace',
  defaultRadius: "0.375rem",
  primaryColor: "amber",
  colors: {
    amber: [
      "#fffcf3",
      "#fbf2d8",
      "#f7e5ad",
      "#f3d782",
      "#efc957",
      "#edb830",
      "#c6a143",
      "#9d7d2f",
      "#735a20",
      "#4a3912",
    ],
  },
});

export interface WorkbenchThemeProps {
  readonly children: ReactNode;
  readonly className?: string;
}

export function WorkbenchTheme({ children, className }: WorkbenchThemeProps): ReactNode {
  const [rootEl, setRootEl] = useState<HTMLDivElement | null>(null);
  const combinedClass = className ? `rl-workbench-root ${className}` : "rl-workbench-root";

  return (
    <div ref={setRootEl} className={combinedClass} data-runlark-workbench="">
      <MantineProvider
        theme={workbenchTheme}
        cssVariablesSelector=".rl-workbench-root"
        getRootElement={() => rootEl ?? undefined}
        defaultColorScheme="light"
        forceColorScheme="light"
      >
        {children}
      </MantineProvider>
    </div>
  );
}
