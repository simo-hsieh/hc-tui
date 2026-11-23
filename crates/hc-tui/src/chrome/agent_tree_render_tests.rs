use std::collections::{BTreeMap, HashSet};

use ratatui::{buffer::Buffer, layout::Rect};

use super::agent_tree::{continuation_prefix, plan, prefix};
use super::{ClientShellConfig, agents, endpoint_agents, render::ShellHitMap};
use crate::project::project;
use crate::sidebar::fixture::{agent, view};

fn shell_config(row_gap: u16) -> ClientShellConfig {
    let mut cfg = ClientShellConfig::from_config(
        &crate::config::Config::default(),
        None,
        None,
        Default::default(),
    );
    cfg.agent_tree = true;
    cfg.agents.row_gap = row_gap;
    cfg.agent_panel_sort = crate::config::AgentPanelSortConfig::Priority;
    cfg
}

fn buffer_rows(buffer: &Buffer, area: Rect) -> Vec<String> {
    (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect()
}

fn make_agent(
    id: &str,
    state: &str,
    title: &str,
    prog: &str,
    parent: Option<&str>,
) -> hc_proto::terminal::Terminal {
    let mut t = agent(id, state);
    t.title = title.into();
    t.labels.insert("herdr.tab".into(), title.into());
    if let Some(status) = t.status.as_mut() {
        status.app = prog.into();
    }
    t.parent = parent.map(Into::into);
    t
}

fn tree_fixture() -> crate::project::Projection {
    let boss = make_agent("t1", "idle", "boss", "claude", None);
    let w1 = make_agent("t2", "working", "w1", "claude", Some("t1"));
    let sub = make_agent("t3", "blocked", "sub", "codex", Some("t2"));
    let w2 = make_agent("t4", "done", "w2", "gemini", Some("t1"));
    let solo = make_agent("t5", "idle", "solo", "claude", None);

    project(&view(vec![boss, w1, sub, w2, solo], vec![]), Some("t1"))
}

#[test]
fn expanded_and_collapsed_single_endpoint_tree_align_cell_by_cell() {
    let projected = tree_fixture();
    let cfg = shell_config(0);
    let area = Rect::new(0, 0, 26, 20);

    // 1. Fully expanded tree: verify connectors, continuation lines, and ▾ toggles.
    let mut expanded_buf = Buffer::empty(area);
    let mut expanded_hits = ShellHitMap::default();
    let mut scroll = 0;
    let mut reveal = false;
    agents::render_agent_panel(
        &mut expanded_buf,
        area,
        &projected.snapshot,
        &cfg,
        &HashSet::new(),
        &mut scroll,
        &mut reveal,
        false,
        &mut expanded_hits,
    );
    let rows = buffer_rows(&expanded_buf, area);
    // Body starts at y = 3.
    assert_eq!(rows[3], " ○ proj · boss          ▾ ");
    assert_eq!(rows[4], " │ claude                 ");
    assert_eq!(rows[5], " ├─● w1                 ▾ ");
    assert_eq!(rows[6], " │ │ claude               ");
    assert_eq!(rows[7], " │ └─● sub                ");
    assert_eq!(rows[8], " │     codex              ");
    assert_eq!(rows[9], " └─● w2                   ");
    assert_eq!(rows[10], "     gemini               ");
    assert_eq!(rows[11], " ○ proj · solo            ");
    assert_eq!(rows[12], "   claude                 ");

    // ▾ toggles sit at col 24 on t1 (y=3) and t2 (y=5), hit rects cover col 24..26.
    assert_eq!(
        expanded_hits.agent_tree_toggles,
        vec![
            (Rect::new(24, 3, 2, 1), "t1".to_string()),
            (Rect::new(24, 5, 2, 1), "t2".to_string()),
        ]
    );

    // 2. Collapse t2 (middle parent): t3 hides, t2 continuation keeps │ for t4.
    let mut collapsed_w1 = HashSet::new();
    collapsed_w1.insert("t2".to_string());
    let mut buf_w1 = Buffer::empty(area);
    let mut hits_w1 = ShellHitMap::default();
    agents::render_agent_panel(
        &mut buf_w1,
        area,
        &projected.snapshot,
        &cfg,
        &collapsed_w1,
        &mut scroll,
        &mut reveal,
        false,
        &mut hits_w1,
    );
    let rows_w1 = buffer_rows(&buf_w1, area);
    assert_eq!(rows_w1[3], " ○ proj · boss          ▾ ");
    assert_eq!(rows_w1[4], " │ claude                 ");
    assert_eq!(rows_w1[5], " ├─● w1          ▸ 1 · 1! ");
    assert_eq!(rows_w1[6], " │   claude               ");
    assert_eq!(rows_w1[7], " └─● w2                   ");
    assert_eq!(rows_w1[8], "     gemini               ");
    assert_eq!(rows_w1[9], " ○ proj · solo            ");
    assert_eq!(rows_w1[10], "   claude                 ");
    assert_eq!(
        hits_w1.agent_tree_toggles,
        vec![
            (Rect::new(24, 3, 2, 1), "t1".to_string()),
            (Rect::new(17, 5, 9, 1), "t2".to_string()),
        ]
    );

    // 3. Collapse t1 (root parent): summary rolls up 1! 1●, continuation drops │.
    let mut collapsed_boss = HashSet::new();
    collapsed_boss.insert("t1".to_string());
    let mut buf_boss = Buffer::empty(area);
    let mut hits_boss = ShellHitMap::default();
    agents::render_agent_panel(
        &mut buf_boss,
        area,
        &projected.snapshot,
        &cfg,
        &collapsed_boss,
        &mut scroll,
        &mut reveal,
        false,
        &mut hits_boss,
    );
    let rows_boss = buffer_rows(&buf_boss, area);
    assert_eq!(rows_boss[3], " ○ proj · bo… ▸ 3 · 1! 1● ");
    assert_eq!(rows_boss[4], "   claude                 ");
    assert_eq!(rows_boss[5], " ○ proj · solo            ");
    assert_eq!(rows_boss[6], "   claude                 ");
    assert_eq!(
        hits_boss.agent_tree_toggles,
        vec![(Rect::new(14, 3, 12, 1), "t1".to_string())]
    );
}

#[test]
fn multi_endpoint_renderer_matches_single_endpoint_tree_alignment() {
    let projected = tree_fixture();
    let cfg = shell_config(0);
    let area = Rect::new(0, 0, 26, 20);
    let mut collapsed = HashSet::new();
    collapsed.insert("t2".to_string());

    let mut single_buf = Buffer::empty(area);
    let mut single_hits = ShellHitMap::default();
    let mut scroll = 0;
    let mut reveal = false;
    agents::render_agent_panel(
        &mut single_buf,
        area,
        &projected.snapshot,
        &cfg,
        &collapsed,
        &mut scroll,
        &mut reveal,
        false,
        &mut single_hits,
    );

    let mut multi_buf = Buffer::empty(area);
    let mut multi_hits = ShellHitMap::default();
    scroll = 0;
    reveal = false;
    endpoint_agents::render_expanded(
        &mut multi_buf,
        area,
        None,
        &projected.endpoints,
        &projected.active_endpoint_id,
        &cfg,
        &collapsed,
        &mut scroll,
        &mut reveal,
        false,
        &mut multi_hits,
    );

    let rows = buffer_rows(&multi_buf, area);
    assert_eq!(rows[3], " ○ local · proj · boss  ▾ ");
    assert_eq!(rows[4], " │ claude                 ");
    assert_eq!(rows[5], " ├─● local · w1  ▸ 1 · 1! ");
    assert_eq!(rows[6], " │   claude               ");
    assert_eq!(rows[7], " └─● local · w2           ");
    assert_eq!(rows[8], "     gemini               ");
    assert_eq!(rows[9], " ○ local · proj · solo    ");
    assert_eq!(rows[10], "   claude                 ");
    assert_eq!(
        multi_hits.agent_tree_toggles,
        single_hits.agent_tree_toggles
    );
}

#[test]
fn tree_mode_suppresses_row_gap_between_parent_and_children() {
    let projected = tree_fixture();
    let cfg = shell_config(1);
    let area = Rect::new(0, 0, 26, 20);
    let mut buf = Buffer::empty(area);
    let mut hits = ShellHitMap::default();
    let mut scroll = 0;
    let mut reveal = false;

    agents::render_agent_panel(
        &mut buf,
        area,
        &projected.snapshot,
        &cfg,
        &HashSet::new(),
        &mut scroll,
        &mut reveal,
        false,
        &mut hits,
    );
    let rows = buffer_rows(&buf, area);
    // No blank gap between t1..t4, but 1 blank gap before the next root family t5.
    assert_eq!(rows[3], " ○ proj · boss          ▾ ");
    assert_eq!(rows[4], " │ claude                 ");
    assert_eq!(rows[5], " ├─● w1                 ▾ ");
    assert_eq!(rows[6], " │ │ claude               ");
    assert_eq!(rows[7], " │ └─● sub                ");
    assert_eq!(rows[8], " │     codex              ");
    assert_eq!(rows[9], " └─● w2                   ");
    assert_eq!(rows[10], "     gemini               ");
    assert_eq!(rows[11], "                          ");
    assert_eq!(rows[12], " ○ proj · solo            ");
}

#[test]
fn clamped_depth_preserves_ancestor_vertical_guide() {
    let parents: BTreeMap<String, String> = [
        ("d1", "r0"),
        ("d2", "d1"),
        ("d3a", "d2"),
        ("d4", "d3a"),
        ("d3b", "d2"),
    ]
    .into_iter()
    .map(|(c, p)| (c.to_string(), p.to_string()))
    .collect();
    let planned = plan(&["r0", "d1", "d2", "d3a", "d4", "d3b"], &parents);
    // d4 is at depth 4 (clamped to visual depth 3) while d3a has sibling d3b at depth 3.
    let d4 = planned[4];
    assert_eq!(d4.depth, 4);
    assert!(d4.last);
    assert_eq!(prefix(d4.depth, d4.last, d4.guides, 26), "    ├─");
    assert_eq!(
        continuation_prefix(d4.depth, d4.last, d4.has_children, d4.guides, 26),
        "    │   "
    );
}
