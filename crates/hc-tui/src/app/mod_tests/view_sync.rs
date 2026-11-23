use super::*;
use hc_proto::watch::Document;
use serde_json::json;

/// `reveal_focused_tab` arms on a focused-tab switch or a tab-order
/// change, and stays quiet when a publish leaves both untouched.
#[tokio::test]
async fn refresh_projection_arms_reveal_only_on_focus_or_order_change() {
    let mut app = app_with_tabs(&[("t1", "space-a", "tab-a"), ("t2", "space-a", "tab-b")]);
    app.focused = Some("t1".into());
    app.refresh_projection();
    app.shell_state.reveal_focused_tab = false;

    // An identical reprojection reveals nothing.
    app.refresh_projection();
    assert!(!app.shell_state.reveal_focused_tab);

    // Focusing a pane on another tab reveals it.
    app.focused = Some("t2".into());
    app.refresh_projection();
    assert!(app.shell_state.reveal_focused_tab);

    // The shared layout document flipping the tab order reveals the
    // focused tab even though the focus itself did not move.
    app.shell_state.reveal_focused_tab = false;
    let workspace_id = app.projection.snapshot.workspaces[0].workspace_id.clone();
    let mut order: Vec<String> = app
        .projection
        .snapshot
        .tabs
        .iter()
        .map(|tab| tab.tab_id.clone())
        .collect();
    order.reverse();
    let key = shared::key();
    let value = json!({
        "schema": 1,
        "order": { crate::project::layout_key(None, &workspace_id): order },
    });
    app.view.documents_mut().insert(
        key,
        Arc::new(Document {
            value,
            version: 3,
            ..Default::default()
        }),
    );
    app.refresh_projection();
    assert!(app.shell_state.reveal_focused_tab);
}

#[tokio::test]
async fn empty_focus_reconcile_still_installs_changed_publish() {
    let mut app = app_with_tabs(&[]);
    assert_eq!(app.focused, None);
    let baseline = app
        .probe
        .projections
        .load(std::sync::atomic::Ordering::Relaxed);

    // An empty-pane publish adds a remote machine: `focus_missing` is
    // true and `fallback` stays `None`, yet the new endpoint must still
    // land in `app.projection` and count one rebuild.
    app.view.version += 1;
    app.view.machines = Some(vec![crate::sidebar::fixture::machine(
        "host-b",
        "connected",
    )]);
    app.reconcile_published_view();
    assert_eq!(app.focused, None);
    assert_eq!(app.projection.endpoints.len(), 2);
    assert_eq!(
        app.probe
            .projections
            .load(std::sync::atomic::Ordering::Relaxed),
        baseline + 1,
    );

    // A bare seq-only republish with `focused == None` runs neither the
    // fallback preview nor a finalizing rebuild.
    app.view.version += 1;
    app.view
        .inventory_mut()
        .apply(&hc_proto::events::EventParams {
            seq: 2,
            kind: "noop".into(),
            terminal: None,
            machine: None,
            data: json!({}),
        });
    app.reconcile_published_view();
    assert_eq!(
        app.probe
            .projections
            .load(std::sync::atomic::Ordering::Relaxed),
        baseline + 1,
    );
}

#[tokio::test]
async fn killed_focused_terminal_falls_back_to_live_sibling() {
    let mut t1 = crate::sidebar::fixture::term("t1");
    t1.labels.insert("herdr.space".into(), "work".into());
    let mut t2 = crate::sidebar::fixture::term("t2");
    t2.labels.insert("herdr.space".into(), "other".into());
    let mut app = app();
    app.open_attach = Arc::new(|_| Box::pin(std::future::pending()));
    app.pane = pane::Pane::new(app.pane_tx.clone(), app.open_attach.clone());
    app.view = crate::sidebar::fixture::view(vec![t1.clone(), t2.clone()], vec![]);
    app.focused = Some("t1".into());
    app.reconcile_published_view();
    assert_eq!(app.focused.as_deref(), Some("t1"));

    // t1 is killed externally while still present in inventory as finished/killed.
    t1.state = "finished".into();
    t1.reason = Some("killed".into());
    app.view = crate::sidebar::fixture::view(vec![t1, t2], vec![]);
    app.view.version += 1;
    app.reconcile_published_view();
    assert_eq!(
        app.focused.as_deref(),
        Some("t2"),
        "focus must move away from a finished/killed terminal"
    );
}
