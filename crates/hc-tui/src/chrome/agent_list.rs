//! Single-endpoint Agents presentation; Spaces keep the original flat order.
use super::agents::{AgentRow, agent_row, ordered_agent_pane_ids};
use super::snapshot::ClientShellSnapshot;
use super::tokens::ResolvedTokenKind;

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

pub(crate) fn row_workspace_label(row: &AgentRow) -> Option<String> {
    row.rows
        .first()?
        .iter()
        .find_map(|token| match &token.kind {
            ResolvedTokenKind::Workspace(label) => Some(label.clone()),
            _ => None,
        })
}

pub(crate) fn compact_tree_child_row(row: &mut AgentRow, parent_workspace: Option<&str>) {
    let Some(first) = row.rows.first_mut() else {
        return;
    };
    let same_workspace = parent_workspace.is_some_and(|parent_ws| {
        first
            .iter()
            .any(|token| matches!(&token.kind, ResolvedTokenKind::Workspace(ws) if ws == parent_ws))
    });
    let has_other_label = first.iter().any(|token| {
        matches!(
            &token.kind,
            ResolvedTokenKind::Tab(_)
                | ResolvedTokenKind::Pane(_)
                | ResolvedTokenKind::Agent(_)
                | ResolvedTokenKind::TerminalTitle(_)
        )
    });
    if same_workspace && has_other_label {
        first.retain(|token| !matches!(&token.kind, ResolvedTokenKind::Workspace(_)));
        row.compact_primary = true;
    }
    if row.rows.len() == 2
        && let [only] = row.rows[1].as_slice()
        && let ResolvedTokenKind::Agent(agent_name) = &only.kind
        && row.rows[0].iter().any(|token| {
            matches!(
                &token.kind,
                ResolvedTokenKind::Tab(label) | ResolvedTokenKind::Pane(label) if label == agent_name
            )
        })
    {
        row.rows.pop();
    }
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
    let workspaces: Vec<_> = rows.iter().map(row_workspace_label).collect();
    let plan = super::agent_tree::plan(&keys, &snapshot.terminal_parents);
    let mut rows: Vec<_> = rows.into_iter().map(Some).collect();
    plan.into_iter()
        .map(|node| {
            let mut row = rows[node.index].take().expect("tree emits each row once");
            row.depth = node.depth;
            row.last_sibling = node.last;
            row.has_children = node.has_children;
            row.guides = node.guides;
            if let Some(parent_idx) = node.parent {
                compact_tree_child_row(&mut row, workspaces[parent_idx].as_deref());
            }
            row
        })
        .collect()
}
