use super::*;
use crate::chrome::{ClientShellConfig, endpoints, render::ShellHitMap};
use ratatui::{buffer::Buffer, layout::Rect};

fn shell(tree: bool) -> ClientShellConfig {
    let mut shell = ClientShellConfig::from_config(
        &crate::config::Config::default(),
        None,
        None,
        Default::default(),
    );
    shell.agent_tree = tree;
    shell.agent_panel_sort = crate::config::AgentPanelSortConfig::Priority;
    shell
}

fn family() -> View {
    let mut child = agent("host-b/t3", "blocked");
    child.parent = Some("t2".into());
    let mut bridge = term("t2");
    bridge.parent = Some("t1".into());
    view(
        vec![agent("t1", "idle"), bridge, child, agent("t4", "working")],
        vec![machine("host-b", "connected")],
    )
}

fn order(projection: &Projection, tree: bool) -> Vec<(String, usize)> {
    endpoints::displayed_agent_rows(
        &projection.endpoints,
        &projection.active_endpoint_id,
        &shell(tree),
    )
    .into_iter()
    .map(|r| (r.agent.pane_id.clone(), r.depth))
    .collect()
}

#[test]
fn tree_groups_cross_machine_worker_through_shell_without_changing_flat_order_or_placement() {
    let view = family();
    let projected = project(&view, Some("host-b/t3"));
    let original = projected.snapshot.clone();
    assert_eq!(
        order(&projected, false),
        [("host-b/t3".into(), 0), ("t4".into(), 0), ("t1".into(), 0)]
    );
    assert_eq!(
        order(&projected, true),
        [("t1".into(), 0), ("host-b/t3".into(), 1), ("t4".into(), 0)]
    );
    assert_eq!(projected.snapshot, original);
    assert_eq!(
        projected
            .endpoints
            .iter()
            .map(|e| e.snapshot.as_ref().unwrap().agents.len())
            .sum::<usize>(),
        3
    );
}

#[test]
fn tree_render_mouse_targets_and_keyboard_indices_agree_at_narrow_widths() {
    let projected = project(&family(), Some("host-b/t3"));
    let config = shell(true);
    let targets = endpoints::online_agent_targets(
        &projected.endpoints,
        &projected.active_endpoint_id,
        &config,
    );
    assert_eq!(
        targets
            .iter()
            .map(|t| (t.pane_id.as_str(), t.row_index))
            .collect::<Vec<_>>(),
        [("t1", 0), ("host-b/t3", 1), ("t4", 2)]
    );
    for width in [4, 8, 18, 40, 64] {
        let area = Rect::new(0, 0, width, 24);
        let mut buffer = Buffer::empty(area);
        let mut hits = ShellHitMap::default();
        let mut scroll = 0;
        let mut reveal = true;
        crate::chrome::endpoint_agents::render_expanded(
            &mut buffer,
            area,
            None,
            &projected.endpoints,
            &projected.active_endpoint_id,
            &config,
            &mut scroll,
            &mut reveal,
            &mut hits,
        );
        assert_eq!(
            hits.endpoint_agents
                .iter()
                .map(|(_, _, id)| id.as_str())
                .collect::<Vec<_>>(),
            ["t1", "host-b/t3", "t4"]
        );
        let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert_eq!(text.contains("└─"), width >= 8);
        let mut hits = ShellHitMap::default();
        crate::chrome::endpoint_agents::render_collapsed(
            &mut buffer,
            area,
            &projected.endpoints,
            &projected.active_endpoint_id,
            &config,
            &mut hits,
        );
        assert_eq!(
            hits.endpoint_agents
                .iter()
                .map(|(_, _, id)| id.as_str())
                .collect::<Vec<_>>(),
            ["t1", "host-b/t3", "t4"]
        );
    }
}

#[test]
fn stale_parent_stays_visible_but_navigation_skips_it() {
    let mut projected = project(&family(), Some("host-b/t3"));
    projected.endpoints[0].status = ClientEndpointStatus::Reconnecting;
    assert_eq!(
        order(&projected, true)
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
        ["t1", "host-b/t3", "t4"]
    );
    let targets = endpoints::online_agent_targets(
        &projected.endpoints,
        &projected.active_endpoint_id,
        &shell(true),
    );
    assert_eq!(
        targets
            .iter()
            .map(|t| (t.pane_id.as_str(), t.row_index))
            .collect::<Vec<_>>(),
        [("host-b/t3", 1)]
    );
}

#[test]
fn relationship_changes_rebuild_tree_even_when_remote_agent_rows_are_cached() {
    let mut view = family();
    let mut cache = crate::project::cache::ProjectCache::default();
    let first = cache.project(&view, Some("host-b/t3"));
    assert_eq!(order(&first, true)[1], ("host-b/t3".into(), 1));
    let mut terms: Vec<_> = view.inventory.terminals().values().cloned().collect();
    terms.iter_mut().find(|t| t.address == "t2").unwrap().parent = None;
    view.inventory_mut().replace(2, terms);
    assert!(!cache.is_unchanged(&view, Some("host-b/t3")));
    let second = cache.project(&view, Some("host-b/t3"));
    assert_eq!(order(&second, true), order(&second, false));
}
