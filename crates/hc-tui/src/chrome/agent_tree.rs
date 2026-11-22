//! A display-only forest over the existing, already ranked agent rows.
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TreeRow {
    pub index: usize,
    pub depth: usize,
    pub last: bool,
}

pub(crate) fn plan(ids: &[&str], parents: &BTreeMap<String, String>) -> Vec<TreeRow> {
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
    let mut remaining: Vec<_> = children[..ids.len()].iter().map(Vec::len).collect();
    let mut leaves: Vec<_> = (0..ids.len()).filter(|&i| remaining[i] == 0).collect();
    while let Some(index) = leaves.pop() {
        if let Some(parent) = links[index] {
            ranks[parent] = ranks[parent].min(ranks[index]);
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
    let push_children = |group: &[usize], depth: usize, stack: &mut Vec<TreeRow>| {
        for (at, &index) in group.iter().enumerate().rev() {
            stack.push(TreeRow {
                index,
                depth,
                last: at + 1 == group.len(),
            });
        }
    };
    push_children(&children[ids.len()], 0, &mut stack);
    let mut rows = Vec::with_capacity(ids.len());
    while let Some(row) = stack.pop() {
        push_children(&children[row.index], row.depth + 1, &mut stack);
        rows.push(row);
    }
    rows
}

pub(crate) fn prefix(depth: usize, last: bool, width: u16) -> String {
    if depth == 0 || width < 8 {
        return String::new();
    }
    let depth = depth.min(3).min(usize::from(width.saturating_sub(6)) / 2);
    if depth == 0 {
        return String::new();
    }
    format!(
        "{}{}",
        "  ".repeat(depth - 1),
        if last { "└─" } else { "├─" }
    )
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
}
