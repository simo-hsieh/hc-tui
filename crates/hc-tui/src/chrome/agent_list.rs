//! Single-endpoint Agents presentation; Spaces keep the original flat order.
use std::collections::HashSet;

use ratatui::{
    style::{Modifier, Style},
    text::Span,
};

use super::agent_tree::TeamSummary;
use super::agents::{AgentRow, agent_row, ordered_agent_pane_ids};
use super::palette::Palette;
use super::render::display_width;
use super::snapshot::ClientShellSnapshot;
use super::status::{AgentStatus, status_color};
use super::tokens::ResolvedTokenKind;

pub(crate) fn displayed_agent_pane_ids(
    snapshot: &ClientShellSnapshot,
    config: &super::ClientShellConfig,
    collapsed: &HashSet<String>,
) -> Vec<String> {
    if !config.agent_tree {
        return ordered_agent_pane_ids(snapshot, config.agent_panel_sort);
    }
    agent_rows(snapshot, config, None, collapsed)
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

pub(crate) struct RowBadge {
    pub spans: Vec<Span<'static>>,
    pub width: u16,
    pub toggle_width: u16,
}

pub(crate) fn tree_row_badge(row: &AgentRow, palette: &Palette) -> Option<RowBadge> {
    let mut spans = Vec::new();
    let status_style = Style::default().fg(status_color(row.status, palette));
    let muted = Style::default().fg(palette.overlay0);

    if row.collapsed && row.summary.total > 0 {
        spans.push(Span::styled("▸", muted));
        append_summary_spans(&mut spans, row.summary, palette);
        let width: u16 = spans.iter().map(|s| display_width(&s.content)).sum();
        return Some(RowBadge {
            spans,
            width,
            toggle_width: width,
        });
    }

    if row.status == AgentStatus::Blocked
        && let Some(kind) = row.blocked_kind.as_deref()
    {
        spans.push(Span::styled(
            super::agent_tree::short_blocked_kind(kind),
            status_style.add_modifier(Modifier::BOLD),
        ));
    } else if matches!(row.status, AgentStatus::Working | AgentStatus::Blocked)
        && let Some(pct) = row.progress
    {
        spans.push(Span::styled(format!("{pct}%"), status_style));
    }

    let mut toggle_width = 0u16;
    if row.has_children {
        if !spans.is_empty() {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled("▾", muted));
        toggle_width = 2;
    }

    if spans.is_empty() {
        return None;
    }
    let width: u16 = spans.iter().map(|s| display_width(&s.content)).sum();
    Some(RowBadge {
        spans,
        width,
        toggle_width: toggle_width.min(width),
    })
}

fn append_summary_spans(spans: &mut Vec<Span<'static>>, summary: TeamSummary, palette: &Palette) {
    let mut parts = 0usize;
    if summary.blocked > 0 {
        spans.push(Span::styled(
            format!(" {}!", summary.blocked),
            Style::default()
                .fg(palette.red)
                .add_modifier(Modifier::BOLD),
        ));
        parts += 1;
    }
    if summary.working > 0 {
        spans.push(Span::styled(
            format!(" {}●", summary.working),
            Style::default().fg(palette.yellow),
        ));
        parts += 1;
    }
    if parts == 0 {
        spans.push(Span::styled(
            format!(" {}", summary.total),
            Style::default().fg(palette.overlay0),
        ));
    }
}

pub(crate) fn agent_rows(
    snapshot: &ClientShellSnapshot,
    config: &super::ClientShellConfig,
    machine: Option<&str>,
    collapsed: &HashSet<String>,
) -> Vec<AgentRow> {
    let rows: Vec<_> = ordered_agent_pane_ids(snapshot, config.agent_panel_sort)
        .into_iter()
        .filter_map(|pane_id| agent_row(snapshot, &pane_id, config, machine))
        .collect();
    if !config.agent_tree {
        return rows;
    }
    let keys: Vec<_> = rows.iter().map(|row| row.pane_id.as_str()).collect();
    let statuses: Vec<_> = rows.iter().map(|row| row.status).collect();
    let workspaces: Vec<_> = rows.iter().map(row_workspace_label).collect();
    let plan = super::agent_tree::plan_with_status(&keys, &statuses, &snapshot.terminal_parents);
    let plan = super::agent_tree::apply_collapse(plan, &keys, collapsed);
    let mut rows: Vec<_> = rows.into_iter().map(Some).collect();
    plan.into_iter()
        .map(|node| {
            let mut row = rows[node.index].take().expect("tree emits each row once");
            row.depth = node.depth;
            row.last_sibling = node.last;
            row.has_children = node.has_children;
            row.collapsed = node.collapsed;
            row.guides = node.guides;
            row.summary = node.summary;
            if let Some(parent_idx) = node.parent {
                compact_tree_child_row(&mut row, workspaces[parent_idx].as_deref());
            }
            row
        })
        .collect()
}
