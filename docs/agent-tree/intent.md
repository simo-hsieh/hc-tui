# Agent worker tree

Show the relationship between independent agent terminals started through `hc run`. Keep the familiar Herdr interface and existing bindings. The tree describes terminal ancestry, not task ownership or built-in harness subagents.

Requirements:

- R1: Flat mode stays the default with unchanged ordering and rendered cells.
- R2: Tree mode uses official core parent links and requires no fork-only capability.
- R3: Existing agent rows appear exactly once. Missing or non-agent parents never hide workers or create fake agent rows.
- R4: Drawing, mouse hits, keyboard targets and focus reveal agree on row order. Offline targets remain non-navigable.
- R5: Cross-machine ancestry, cycles and deep chains cannot hang the UI or drop rows.
- R6: Tree rendering changes no process lifetime, status, placement, labels, layout documents or focus ownership.
- R7: Narrow, mobile and collapsed lists stay usable.

Success means a local agent and its remote worker appear together, navigation targets each real terminal, and omitting the option restores the flat list.

Collapse state, new bindings, task orchestration, persistent preferences, control IPC, status inference and shared-focus changes are outside scope.
