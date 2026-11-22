use super::*;
use crate::input::KeybindAction as A;
use crate::sidebar::fixture::agent;

#[tokio::test]
async fn local_tree_render_navigation_and_indexed_focus_agree_expanded_and_collapsed() {
    let mut child = agent("t2", "blocked");
    child.parent = Some("t1".into());
    let mut second_child = agent("t4", "working");
    second_child.parent = Some("t3".into());
    let mut app = endpoint_nav::app_multi(
        vec![
            agent("t1", "idle"),
            child,
            agent("t3", "idle"),
            second_child,
        ],
        vec![],
    );
    endpoint_nav::pending_attach(&mut app);
    app.shell.agent_tree = true;
    app.shell.agent_panel_sort = crate::config::AgentPanelSortConfig::Priority;
    app.focused = Some("t1".into());
    app.refresh_projection();
    let traversal = crate::project::agent_traversal(&app.view);
    let orders: Vec<_> = app
        .projection
        .endpoints
        .iter()
        .map(|e| e.snapshot.as_ref().unwrap().agent_order.clone())
        .collect();
    let expected = ["t1", "t2", "t3", "t4"];
    for collapsed in [false, true] {
        app.shell_state.sidebar_collapsed = collapsed;
        let mut terminal = Terminal::new(TestBackend::new(120, 34)).unwrap();
        terminal
            .draw(|f| draw::draw(&mut app, f, Instant::now()))
            .unwrap();
        assert_eq!(
            app.hit_map
                .agents
                .iter()
                .map(|(_, id)| id.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        app.focused = Some("t1".into());
        app.refresh_projection();
        app.on_action(A::NextAgent, 120);
        assert_eq!(app.focused.as_deref(), Some("t2"));
        app.on_action(A::NextAgent, 120);
        assert_eq!(
            app.focused.as_deref(),
            Some("t3"),
            "tree visits the next family; flat order would visit t4"
        );
        app.on_action(A::FocusAgent(1), 120);
        assert_eq!(app.focused.as_deref(), Some("t2"));
    }
    app.shell.agent_tree = false;
    app.refresh_projection();
    assert_eq!(crate::project::agent_traversal(&app.view), traversal);
    assert_eq!(
        app.projection
            .endpoints
            .iter()
            .map(|e| e.snapshot.as_ref().unwrap().agent_order.clone())
            .collect::<Vec<_>>(),
        orders
    );
}
