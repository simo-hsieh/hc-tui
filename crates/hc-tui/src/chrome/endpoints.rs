// Derived from herdr src/client/shell/endpoint_sidebar.rs,
// src/client/shell/endpoints.rs, and src/client/shell/aggregate_navigation.rs
// (Apache-2.0).

use std::collections::HashSet;

use super::palette::Palette;
use super::render::ShellRenderState;
use super::snapshot::{
    ClientEndpointId, ClientEndpointStatus, ClientShellEndpoint, ClientShellSnapshot,
};
use super::status::status_priority;

pub(crate) fn endpoint_status_presentation(
    status: ClientEndpointStatus,
    palette: &Palette,
) -> (&'static str, &'static str, ratatui::style::Color) {
    match status {
        ClientEndpointStatus::Connecting => ("◐", "connecting", palette.yellow),
        ClientEndpointStatus::Online => ("●", "online", palette.green),
        ClientEndpointStatus::Reconnecting => ("◐", "reconnecting", palette.yellow),
        ClientEndpointStatus::Attention => ("!", "attention", palette.red),
        ClientEndpointStatus::Disabled => ("·", "disabled", palette.overlay0),
    }
}

#[derive(Clone, Copy)]
pub(crate) struct CachedEndpointSnapshot<'a> {
    pub(super) endpoint_id: &'a ClientEndpointId,
    pub(super) label: &'a str,
    pub(super) status: ClientEndpointStatus,
    pub(super) snapshot: &'a ClientShellSnapshot,
    /// The endpoint's own recency map (pane_id → observed change order).
    pub(super) agent_recency: &'a std::collections::HashMap<String, u64>,
}

impl CachedEndpointSnapshot<'_> {
    pub(super) fn stale(self) -> bool {
        self.status != ClientEndpointStatus::Online
    }
}

pub(crate) fn cached_endpoint_snapshots(
    endpoints: &[ClientShellEndpoint],
) -> impl Iterator<Item = CachedEndpointSnapshot<'_>> {
    endpoints.iter().filter_map(|endpoint| {
        endpoint
            .snapshot
            .as_deref()
            .map(|snapshot| CachedEndpointSnapshot {
                endpoint_id: &endpoint.endpoint_id,
                label: &endpoint.label,
                status: endpoint.status,
                snapshot,
                agent_recency: &endpoint.agent_recency,
            })
    })
}

pub(crate) struct AggregateAgentRow<'a> {
    pub endpoint: CachedEndpointSnapshot<'a>,
    pub agent: &'a super::snapshot::ClientShellAgent,
    pub recency: u64,
    pub depth: usize,
    pub last_sibling: bool,
    pub has_children: bool,
    pub guides: u8,
    pub parent_workspace: Option<&'a str>,
}

/// Endpoint-ordered agent rows. `agent_view` filtering is not part of hc-tui's
/// M2 projection, so this is the reference's non-view path.
pub(crate) fn aggregate_agent_rows<'a>(
    endpoints: &'a [ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
    sort: crate::config::AgentPanelSortConfig,
) -> Vec<AggregateAgentRow<'a>> {
    let mut rows = cached_endpoint_snapshots(endpoints)
        .flat_map(|endpoint| {
            super::agents::ordered_agent_pane_ids(endpoint.snapshot, sort)
                .into_iter()
                .filter_map(move |pane_id| {
                    let agent = endpoint
                        .snapshot
                        .agents
                        .iter()
                        .find(|agent| agent.pane_id == pane_id)?;
                    Some(AggregateAgentRow {
                        recency: endpoint.agent_recency.get(&pane_id).copied().unwrap_or(0),
                        endpoint,
                        agent,
                        depth: 0,
                        last_sibling: true,
                        has_children: false,
                        guides: 0,
                        parent_workspace: None,
                    })
                })
        })
        .collect::<Vec<_>>();
    if sort == crate::config::AgentPanelSortConfig::Priority {
        rows.sort_by_key(|row| {
            (
                row.endpoint.stale(),
                std::cmp::Reverse(status_priority(row.agent.agent_status)),
                std::cmp::Reverse(row.recency),
            )
        });
    }
    let _ = active_endpoint_id;
    rows
}

/// The Agents view's final order, shared by renderers and navigation.
pub(crate) fn displayed_agent_rows<'a>(
    endpoints: &'a [ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
    config: &super::ClientShellConfig,
) -> Vec<AggregateAgentRow<'a>> {
    let mut rows = aggregate_agent_rows(endpoints, active_endpoint_id, config.agent_panel_sort);
    if !config.agent_tree {
        return rows;
    }
    rows.retain(|row| {
        row.endpoint
            .snapshot
            .workspaces
            .iter()
            .any(|w| w.workspace_id == row.agent.workspace_id)
    });
    let ids: Vec<_> = rows.iter().map(|row| row.agent.pane_id.as_str()).collect();
    let workspaces: Vec<Option<&str>> = rows
        .iter()
        .map(|row| {
            row.endpoint
                .snapshot
                .workspaces
                .iter()
                .find(|w| w.workspace_id == row.agent.workspace_id)
                .map(|w| w.label.as_str())
        })
        .collect();
    // Projection gives every endpoint the same full-inventory relationship Arc.
    let parents = endpoints
        .iter()
        .find_map(|e| e.snapshot.as_ref())
        .map(|s| s.terminal_parents.as_ref());
    let Some(parents) = parents else {
        return rows;
    };
    let plan = super::agent_tree::plan(&ids, parents);
    let mut rows: Vec<_> = rows.into_iter().map(Some).collect();
    plan.into_iter()
        .map(|node| {
            let mut row = rows[node.index].take().expect("tree emits each row once");
            row.depth = node.depth;
            row.last_sibling = node.last;
            row.has_children = node.has_children;
            row.guides = node.guides;
            row.parent_workspace = node.parent.and_then(|parent_idx| workspaces[parent_idx]);
            row
        })
        .collect()
}

/// One row of `aggregate_agent_rows` usable as a navigation target:
/// its endpoint, its pane, and its position in the rendered
/// (unfiltered) list so reveal can scroll to the row the sidebar drew.
#[derive(Clone)]
pub(crate) struct AggregateAgentTarget {
    pub endpoint_id: ClientEndpointId,
    pub pane_id: String,
    /// Index in `aggregate_agent_rows` — stale endpoints' rows still
    /// render, so this is the position the sidebar scrolls to.
    pub row_index: usize,
}

/// The agent rows previous/next/focus agent navigation may land on:
/// `aggregate_agent_rows` reduced to endpoints whose transport is
/// Online. Derived from `aggregate_navigation.rs::online_agent_targets`.
pub(crate) fn online_agent_targets(
    endpoints: &[ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
    config: &super::ClientShellConfig,
) -> Vec<AggregateAgentTarget> {
    displayed_agent_rows(endpoints, active_endpoint_id, config)
        .into_iter()
        .enumerate()
        .filter(|(_, row)| row.endpoint.status == ClientEndpointStatus::Online)
        .map(|(row_index, row)| AggregateAgentTarget {
            endpoint_id: row.endpoint.endpoint_id.clone(),
            pane_id: row.agent.pane_id.clone(),
            row_index,
        })
        .collect()
}

pub(crate) fn collapsed_groups_for_endpoint<'a>(
    state: &'a ShellRenderState<'_>,
    endpoint_id: &ClientEndpointId,
) -> Option<&'a HashSet<String>> {
    if endpoint_id.is_local() {
        Some(state.collapsed_groups)
    } else {
        state.remote_collapsed_groups.get(endpoint_id)
    }
}
