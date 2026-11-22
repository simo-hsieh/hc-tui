//! A display-only forest over the existing, already ranked agent rows.
use std::collections::{BTreeMap, HashSet};

use super::status::AgentStatus;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TeamSummary {
    pub total: usize,
    pub blocked: usize,
    pub working: usize,
    pub done: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TreeRow {
    pub index: usize,
    pub depth: usize,
    pub last: bool,
    pub parent: Option<usize>,
    pub has_children: bool,
    pub collapsed: bool,
    pub guides: u8,
    pub summary: TeamSummary,
}

const MAX_VISUAL_DEPTH: usize = 3;

#[cfg(test)]
pub(crate) fn plan(ids: &[&str], parents: &BTreeMap<String, String>) -> Vec<TreeRow> {
    let statuses = vec![AgentStatus::Idle; ids.len()];
    plan_with_status(ids, &statuses, parents)
}

pub(crate) fn plan_with_status(
    ids: &[&str],
    statuses: &[AgentStatus],
    parents: &BTreeMap<String, String>,
) -> Vec<TreeRow> {
    let positions: BTreeMap<_, _> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let mut links: Vec<Option<usize>> = ids
        .iter()
        .map(|id| {
            let mut next = parents.get(*id);
            let mut seen = std::collections::BTreeSet::new();
            while let Some(parent) = next {
                if !seen.insert(parent) {
                    break;
                }
                if let Some(&index) = positions.get(parent.as_str()) {
                    return Some(index);
                }
                next = parents.get(parent);
            }
            None
        })
        .collect();
    // Compression through invisible terminals can itself produce a cycle.
    let mut state = vec![0u8; ids.len()];
    for start in 0..ids.len() {
        let mut path = Vec::new();
        let mut next = Some(start);
        while let Some(index) = next {
            if state[index] != 0 {
                if state[index] == 1 {
                    let at = path.iter().position(|&i| i == index).expect("current path");
                    let root = *path[at..].iter().min_by_key(|&&i| ids[i]).expect("cycle");
                    links[root] = None;
                }
                break;
            }
            state[index] = 1;
            path.push(index);
            next = links[index];
        }
        for index in path {
            state[index] = 2;
        }
    }
    let mut children = vec![Vec::new(); ids.len() + 1];
    for (index, parent) in links.iter().enumerate() {
        children[parent.unwrap_or(ids.len())].push(index);
    }
    let mut ranks: Vec<_> = (0..ids.len()).collect();
    let mut summaries = vec![TeamSummary::default(); ids.len()];
    let mut remaining: Vec<_> = children[..ids.len()].iter().map(Vec::len).collect();
    let mut leaves: Vec<_> = (0..ids.len()).filter(|&i| remaining[i] == 0).collect();
    while let Some(index) = leaves.pop() {
        if let Some(parent) = links[index] {
            ranks[parent] = ranks[parent].min(ranks[index]);
            let child_status = statuses.get(index).copied().unwrap_or(AgentStatus::Unknown);
            let child_summary = summaries[index];
            summaries[parent].total += 1 + child_summary.total;
            summaries[parent].blocked +=
                usize::from(child_status == AgentStatus::Blocked) + child_summary.blocked;
            summaries[parent].working +=
                usize::from(child_status == AgentStatus::Working) + child_summary.working;
            summaries[parent].done +=
                usize::from(child_status == AgentStatus::Done) + child_summary.done;
            remaining[parent] -= 1;
            if remaining[parent] == 0 {
                leaves.push(parent);
            }
        }
    }
    for group in &mut children {
        group.sort_by_key(|&index| ranks[index]);
    }
    let mut stack = Vec::new();
    let push_children = |group: &[usize],
                         parent: Option<usize>,
                         depth: usize,
                         guides: u8,
                         children: &[Vec<usize>],
                         summaries: &[TeamSummary],
                         stack: &mut Vec<TreeRow>| {
        for (at, &index) in group.iter().enumerate().rev() {
            stack.push(TreeRow {
                index,
                depth,
                last: at + 1 == group.len(),
                parent,
                has_children: !children[index].is_empty(),
                collapsed: false,
                guides,
                summary: summaries[index],
            });
        }
    };
    push_children(
        &children[ids.len()],
        None,
        0,
        0,
        &children,
        &summaries,
        &mut stack,
    );
    let mut rows = Vec::with_capacity(ids.len());
    while let Some(row) = stack.pop() {
        let child_guides = if row.depth > 0 && !row.last && row.depth - 1 < 8 {
            row.guides | (1u8 << (row.depth - 1))
        } else {
            row.guides
        };
        push_children(
            &children[row.index],
            Some(row.index),
            row.depth + 1,
            child_guides,
            &children,
            &summaries,
            &mut stack,
        );
        rows.push(row);
    }
    rows
}

pub(crate) fn apply_collapse(
    rows: Vec<TreeRow>,
    ids: &[&str],
    collapsed: &HashSet<String>,
) -> Vec<TreeRow> {
    if collapsed.is_empty() {
        return rows;
    }
    let mut visible = Vec::with_capacity(rows.len());
    let mut hidden_below: Option<usize> = None;
    for mut row in rows {
        if let Some(depth) = hidden_below {
            if row.depth > depth {
                continue;
            }
            hidden_below = None;
        }
        if row.has_children && collapsed.contains(ids[row.index]) {
            row.collapsed = true;
            hidden_below = Some(row.depth);
        }
        visible.push(row);
    }
    visible
}

pub(crate) fn short_blocked_kind(kind: &str) -> String {
    match kind {
        "permission" => "perm".to_string(),
        "question" => "ask".to_string(),
        "auth" => "auth".to_string(),
        other => other.chars().take(4).collect(),
    }
}

fn visible_depth(depth: usize, width: u16) -> usize {
    if depth == 0 || width < 8 {
        return 0;
    }
    depth
        .min(MAX_VISUAL_DEPTH)
        .min(usize::from(width.saturating_sub(6)) / 2)
}

pub(crate) fn prefix(depth: usize, last: bool, guides: u8, width: u16) -> String {
    let depth = visible_depth(depth, width);
    if depth == 0 {
        return String::new();
    }
    let mut out = String::with_capacity(depth * 2);
    for level in 0..(depth - 1) {
        if (guides & (1u8 << level)) != 0 {
            out.push_str("│ ");
        } else {
            out.push_str("  ");
        }
    }
    out.push_str(if last { "└─" } else { "├─" });
    out
}

pub(crate) fn continuation_prefix(
    depth: usize,
    last: bool,
    has_children: bool,
    guides: u8,
    width: u16,
) -> String {
    let d = visible_depth(depth, width);
    if depth == 0 {
        return if has_children && width >= 8 {
            "│ ".to_string()
        } else {
            "  ".to_string()
        };
    }
    if d == 0 {
        return "  ".to_string();
    }
    let mut out = String::with_capacity((d + 1) * 2);
    for level in 0..(d - 1) {
        if (guides & (1u8 << level)) != 0 {
            out.push_str("│ ");
        } else {
            out.push_str("  ");
        }
    }
    out.push_str(if last { "  " } else { "│ " });
    let child_visible = has_children && visible_depth(depth + 1, width) > d;
    out.push_str(if child_visible { "│ " } else { "  " });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(ids: &[&str], edges: &[(&str, &str)]) -> Vec<(usize, usize)> {
        let parents = edges
            .iter()
            .map(|(child, parent)| (child.to_string(), parent.to_string()))
            .collect();
        plan(ids, &parents)
            .into_iter()
            .map(|row| (row.index, row.depth))
            .collect()
    }

    #[test]
    fn parent_precedes_urgent_child_and_family_keeps_best_rank() {
        assert_eq!(
            rows(&["child", "other", "parent"], &[("child", "parent")]),
            [(2, 0), (0, 1), (1, 0)]
        );
    }

    #[test]
    fn chains_cross_hidden_shells_and_remote_endpoints() {
        assert_eq!(
            rows(
                &["host-b/t9", "t1"],
                &[("host-b/t9", "shell"), ("shell", "t1")]
            ),
            [(1, 0), (0, 1)]
        );
    }

    #[test]
    fn unknown_alias_and_missing_parent_keep_rows_as_roots() {
        assert_eq!(
            rows(&["t1", "t2"], &[("t1", "unknown/t2"), ("t2", "missing")]),
            [(0, 0), (1, 0)]
        );
    }

    #[test]
    fn cycle_through_hidden_nodes_breaks_at_smallest_id() {
        assert_eq!(
            rows(
                &["b", "a"],
                &[("a", "x"), ("x", "b"), ("b", "y"), ("y", "a")]
            ),
            [(1, 0), (0, 1)]
        );
    }

    #[test]
    fn hidden_cycle_and_self_parent_keep_every_row() {
        assert_eq!(
            rows(
                &["a", "b"],
                &[("a", "x"), ("x", "y"), ("y", "x"), ("b", "b")]
            ),
            [(0, 0), (1, 0)]
        );
    }

    #[test]
    fn independent_shell_children_remain_independent_roots() {
        assert_eq!(
            rows(&["a", "b"], &[("a", "shell"), ("b", "shell")]),
            [(0, 0), (1, 0)]
        );
    }

    #[test]
    fn deep_tree_is_iterative_and_never_drops_rows() {
        let names: Vec<_> = (0..2048).map(|i| format!("t{i}")).collect();
        let parents = (1..names.len())
            .map(|i| (names[i].clone(), names[i - 1].clone()))
            .collect();
        let ids: Vec<_> = names.iter().map(String::as_str).rev().collect();
        let actual = plan(&ids, &parents);
        assert_eq!(actual.len(), names.len());
        assert_eq!(actual.last().unwrap().depth, names.len() - 1);
    }

    #[test]
    fn guides_and_continuation_prefixes_connect_siblings_and_children() {
        let parents = [("w1", "boss"), ("sub", "w1"), ("w2", "boss")]
            .into_iter()
            .map(|(c, p)| (c.to_string(), p.to_string()))
            .collect();
        let planned = plan(&["boss", "w1", "sub", "w2"], &parents);
        // boss (depth 0, has_children) -> w1 (depth 1, !last, has_children)
        //   -> sub (depth 2, last, guides bit 0 set) -> w2 (depth 1, last)
        assert_eq!(
            prefix(planned[0].depth, planned[0].last, planned[0].guides, 26),
            ""
        );
        assert_eq!(
            continuation_prefix(
                planned[0].depth,
                planned[0].last,
                planned[0].has_children,
                planned[0].guides,
                26
            ),
            "│ "
        );
        assert_eq!(
            prefix(planned[1].depth, planned[1].last, planned[1].guides, 26),
            "├─"
        );
        assert_eq!(
            continuation_prefix(
                planned[1].depth,
                planned[1].last,
                planned[1].has_children,
                planned[1].guides,
                26
            ),
            "│ │ "
        );
        assert_eq!(
            prefix(planned[2].depth, planned[2].last, planned[2].guides, 26),
            "│ └─"
        );
        assert_eq!(
            continuation_prefix(
                planned[2].depth,
                planned[2].last,
                planned[2].has_children,
                planned[2].guides,
                26
            ),
            "│     "
        );
        assert_eq!(
            prefix(planned[3].depth, planned[3].last, planned[3].guides, 26),
            "└─"
        );
        assert_eq!(
            continuation_prefix(
                planned[3].depth,
                planned[3].last,
                planned[3].has_children,
                planned[3].guides,
                26
            ),
            "    "
        );
    }

    #[test]
    fn team_summary_rolls_up_descendants_and_collapse_hides_subtree() {
        let ids = ["boss", "w1", "sub", "w2", "solo"];
        let statuses = [
            AgentStatus::Idle,
            AgentStatus::Working,
            AgentStatus::Blocked,
            AgentStatus::Done,
            AgentStatus::Idle,
        ];
        let parents = [("w1", "boss"), ("sub", "w1"), ("w2", "boss")]
            .into_iter()
            .map(|(c, p)| (c.to_string(), p.to_string()))
            .collect();
        let planned = plan_with_status(&ids, &statuses, &parents);
        assert_eq!(
            planned[0].summary,
            TeamSummary {
                total: 3,
                blocked: 1,
                working: 1,
                done: 1,
            }
        );
        let collapsed: HashSet<String> = ["boss".to_string()].into_iter().collect();
        let visible = apply_collapse(planned, &ids, &collapsed);
        assert_eq!(
            visible.iter().map(|r| ids[r.index]).collect::<Vec<_>>(),
            ["boss", "solo"]
        );
        assert!(visible[0].collapsed);
        assert_eq!(visible[0].summary.blocked, 1);
        assert_eq!(short_blocked_kind("permission"), "perm");
        assert_eq!(short_blocked_kind("question"), "ask");
    }
}
