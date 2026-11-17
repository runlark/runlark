---
feod-profile: 1
---

# FEOD profile

> Overrides the defaults of the feod skill. To change it, run the skill's init flow again.

| Parameter                           | Value                                       |
| ----------------------------------- | ------------------------------------------- |
| modification                        | base                                        |
| multiapp                            | no                                          |
| framework                           | react (mantine v9, reatom v1001, dsh slots) |
| srcRoot / alias                     | web/src / @                                 |
| layers                              | app, pages, modules, common, global         |
| pages                               | keep                                        |
| modulePages                         | yes                                         |
| fileBasedRouting                    | no (manual router)                          |
| privatePageModules (`pages/_name/`) | allow                                       |
| common.allowIndex                   | false                                       |
| modules.allowDeepImports            | false                                       |
| singleFileModules                   | no                                          |
| transitModules                      | none                                        |
| uiKit                               | common/ui (project override)                |
| layouts                             | app only                                    |
| ioc                                 | required for cross-module links             |
| ddd                                 | no                                          |
| lint                                | oxlint + @o-feod/oxlint-structure-plugin    |

## Project notes

- Only `web/src/common/ui/<component>` adapters import `@mantine/*`; consumers take readonly props/slots contracts (`web/DESIGN.md`).
- State and async flows use Reatom v1001 (`@reatom/core`, `reatom-async`) with React/Mantine adapters; `@reatom/jsx` is forbidden.
- Production Task-to-PR views live in `web/src/{app,pages,modules/task-flow}`; `web/stories/` is Storybook-only.
- DSH UI slots (`ui-dockkit`, `ui-sidebar-right`, `tool.call.toolview`) mount `web/src/app` via `packages/dsh-plugin/src/ui`.
