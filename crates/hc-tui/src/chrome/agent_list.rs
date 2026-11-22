//! Single-endpoint Agents presentation; Spaces keep the original flat order.
use super::agents::{AgentRow, agent_row, ordered_agent_pane_ids};
use super::snapshot::ClientShellSnapshot;

pub(crate) fn displayed_agent_pane_ids(
    snapshot: &ClientShellSnapshot,
    config: &super::ClientShellConfig,
) -> Vec<String> {
    if !config.agent_tree {
        return ordered_agent_pane_ids(snapshot, config.agent_panel_sort);
    }
    agent_rows(snapshot, config, None)
        .into_iter()
        .map(|row| row.pane_id)
        .collect()
}

pub(crate) fn agent_rows(
    snapshot: &ClientShellSnapshot,
    config: &super::ClientShellConfig,
    machine: Option<&str>,
) -> Vec<AgentRow> {
    let rows: Vec<_> = ordered_agent_pane_ids(snapshot, config.agent_panel_sort)
        .into_iter()
        .filter_map(|pane_id| agent_row(snapshot, &pane_id, config, machine))
        .collect();
    if !config.agent_tree {
        return rows;
    }
    let keys: Vec<_> = rows.iter().map(|row| row.pane_id.as_str()).collect();
    let plan = super::agent_tree::plan(&keys, &snapshot.terminal_parents);
    let mut rows: Vec<_> = rows.into_iter().map(Some).collect();
    plan.into_iter()
        .map(|node| {
            let mut row = rows[node.index].take().expect("tree emits each row once");
            row.depth = node.depth;
            row.last_sibling = node.last;
            row
        })
        .collect()
}
