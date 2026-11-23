use super::*;
use crate::app::input::{MouseButton, MouseEventKind};
use crate::chrome::menu::{ContextMenuAction, ContextMenuTarget};
use crate::chrome::render::{ClientMobileTarget, MachineHit, PaneHit};
use crate::input::KeybindAction as A;
use crate::testcore;
use hc_vt::{Action, Key, Mods};
use ratatui::layout::Rect;
use serde_json::json;

fn pending(app: &mut App) {
    app.open_attach = Arc::new(|_| Box::pin(std::future::pending()));
    app.pane = pane::Pane::new(app.pane_tx.clone(), app.open_attach.clone());
}

/// Routes layout actions at a fake core answering as `core-fake-1`.
fn operations(app: &mut App) -> mpsc::Receiver<testcore::FakeCore> {
    let (accepted, rx) = mpsc::channel(4);
    let (rpc, _sub) = testcore::openers(accepted);
    app.set_operations(rpc);
    app.view.core.as_mut().unwrap().id = "core-fake-1".into();
    rx
}

async fn settle(app: &mut App) {
    let result = tokio::time::timeout(
        testcore::DEADLINE,
        App::operation_result(&mut app.operations),
    )
    .await
    .unwrap();
    app.on_operation(result);
}

fn press(key: Key) -> input::TerminalKey {
    term_key(key, Mods::NONE, Action::Press)
}

fn clear_rename(app: &mut App) {
    app.dialog_key(&term_key(Key::Char('c'), Mods::CTRL, Action::Press));
}

#[tokio::test]
async fn a_workspace_right_click_opens_the_reference_menu_and_rename_writes_the_adoption() {
    let mut app = app_with_spaces(&[("t1", "work"), ("t2", "work")]);
    pending(&mut app);
    draw_to(&mut app, 120, 40);
    let hit = app
        .hit_map
        .workspaces
        .iter()
        .find(|h| h.endpoint_id == ClientEndpointId::Local)
        .unwrap()
        .clone();
    app.context_menu_press((hit.rect.x, hit.rect.y));
    let Some(menu) = &app.menu else {
        panic!("the workspace row opens a context menu")
    };
    let ContextMenuTarget::Workspace { workspace_id, .. } = &menu.target else {
        panic!("the menu targets the clicked workspace")
    };
    assert_eq!(workspace_id, &hit.workspace_id);
    let labels: Vec<_> = app
        .menu
        .as_ref()
        .unwrap()
        .items()
        .iter()
        .map(|i| i.label)
        .collect();
    assert_eq!(labels[..2], ["Rename", "Close"]);
    let mut rx = operations(&mut app);
    let server = tokio::spawn(async move {
        let mut core = rx.recv().await.unwrap();
        core.read_hello().await;
        core.answer_hello(true, &["doc.get", "doc.set"], &[]).await;
        let read = core.read_request().await;
        core.reply(&read["id"], json!({"value":null,"version":0}))
            .await;
        let write = core.read_request().await;
        assert_eq!(write["method"], "doc.set");
        let adoption = &write["params"]["value"]["adoptions"]["0:/space:4:work"];
        assert_eq!(adoption["name"], "renamed");
        assert_eq!(adoption["custom_label"], true);
        core.reply(&write["id"], json!({"version":1})).await;
    });
    // Item 0 is "Rename": Enter opens the rename dialog.
    app.menu_key(&press(Key::Enter));
    let Some(dialog) = &app.dialog else {
        panic!("rename opens a dialog")
    };
    let super::super::dialogs::Target::Workspace { .. } = dialog.target else {
        panic!("the dialog persists a workspace adoption")
    };
    clear_rename(&mut app);
    app.insert_dialog_text("renamed");
    app.dialog_key(&press(Key::Enter));
    settle(&mut app).await;
    assert_eq!(app.pending_actions, 0);
    assert!(app.notice.is_none());
    server.await.unwrap();
}

#[tokio::test]
async fn a_machine_connect_activation_reaches_the_wire_without_a_dialog() {
    let mut app = app_with_spaces(&[("t1", "work")]);
    pending(&mut app);
    app.hit_map.machines.push(MachineHit {
        rect: Rect::new(0, 0, 12, 1),
        status_badge: Rect::default(),
        collapse_toggle: Rect::default(),
        new_workspace: Rect::default(),
        endpoint_id: ClientEndpointId::Remote("host-b".into()),
    });
    app.context_menu_press((1, 0));
    let Some(menu) = &app.menu else {
        panic!("the machine row opens a context menu")
    };
    let ContextMenuTarget::Machine { name } = &menu.target else {
        panic!("the menu targets the machine")
    };
    assert_eq!(name.as_deref(), Some("host-b"));
    let actions: Vec<_> = app
        .menu
        .as_ref()
        .unwrap()
        .items()
        .iter()
        .map(|i| i.action)
        .collect();
    assert_eq!(
        actions,
        [
            ContextMenuAction::MachineConnect,
            ContextMenuAction::MachineRename,
            ContextMenuAction::MachineRemove,
        ]
    );
    let mut rx = operations(&mut app);
    let server = tokio::spawn(async move {
        let mut core = rx.recv().await.unwrap();
        core.read_hello().await;
        core.answer_hello(true, &["machine.connect"], &[]).await;
        let req = core.read_request().await;
        assert_eq!(req["method"], "machine.connect");
        assert_eq!(req["params"]["machine"], "host-b");
        core.reply(&req["id"], json!({})).await;
    });
    app.menu_key(&press(Key::Enter));
    assert!(app.menu.is_none());
    settle(&mut app).await;
    assert_eq!(app.pending_actions, 0);
    assert!(app.notice.is_none());
    server.await.unwrap();
}

#[tokio::test]
async fn machine_dialogs_submit_add_rename_and_remove() {
    use super::super::dialogs::Target;
    for (open, method, params) in [
        ("add", "machine.add", json!({"target": "alice@host-b"})),
        (
            "rename",
            "machine.rename",
            json!({"machine": "host-b", "new_name": "build-box"}),
        ),
        ("remove", "machine.rm", json!({"machine": "host-b"})),
    ] {
        let mut app = app_with_spaces(&[("t1", "work")]);
        pending(&mut app);
        let mut rx = operations(&mut app);
        let server = tokio::spawn(async move {
            let mut core = rx.recv().await.unwrap();
            core.read_hello().await;
            core.answer_hello(true, &[method], &[]).await;
            let req = core.read_request().await;
            assert_eq!(req["method"], method);
            assert_eq!(req["params"], params);
            core.reply(&req["id"], json!({})).await;
        });
        match open {
            "add" => {
                app.machine_add();
                assert!(matches!(
                    app.dialog.as_ref().unwrap().target,
                    Target::MachineAdd
                ));
                app.insert_dialog_text("alice@host-b");
            }
            "rename" => {
                app.machine_rename("host-b".into());
                assert!(matches!(
                    app.dialog.as_ref().unwrap().target,
                    Target::MachineRename { .. }
                ));
                // The prefilled name is selected: typing replaces it.
                app.insert_dialog_text("build-box");
            }
            _ => {
                app.machine_remove("host-b".into());
                assert!(matches!(
                    app.dialog.as_ref().unwrap().target,
                    Target::MachineRemove { .. }
                ));
            }
        }
        app.dialog_key(&press(Key::Enter));
        settle(&mut app).await;
        assert_eq!(app.pending_actions, 0, "{open}");
        assert!(app.notice.is_none(), "{open}");
        server.await.unwrap();
    }
}

#[tokio::test]
async fn new_workspace_runs_a_kept_terminal_and_records_the_adoption() {
    let mut app = app_with_spaces(&[("t1", "work"), ("t2", "work")]);
    pending(&mut app);
    app.focused = Some("t1".into());
    app.refresh_projection();
    let mut rx = operations(&mut app);
    let server = tokio::spawn(async move {
        let mut core = rx.recv().await.unwrap();
        core.read_hello().await;
        core.answer_hello(true, &["doc.get", "doc.set", "terminal.run"], &[])
            .await;
        for _ in 0..2 {
            let read = core.read_request().await;
            assert_eq!(read["method"], "doc.get");
            core.reply(&read["id"], json!({"value":null,"version":0}))
                .await;
        }
        let run = core.read_request().await;
        assert_eq!(run["method"], "terminal.run");
        assert_eq!(run["params"]["keep"], true);
        assert_eq!(run["params"]["no_parent"], true);
        assert_eq!(run["params"]["no_notify"], true);
        assert_eq!(run["params"]["labels"]["herdr.seen"], "0");
        assert!(
            !run["params"]["labels"]["herdr.space"]
                .as_str()
                .unwrap()
                .is_empty()
        );
        let mut terminal = testcore::minimal_terminal();
        terminal["address"] = json!("t9");
        core.reply(&run["id"], terminal).await;
        let read = core.read_request().await;
        core.reply(&read["id"], json!({"value":null,"version":0}))
            .await;
        let write = core.read_request().await;
        assert_eq!(write["method"], "doc.set");
        let adoptions = write["params"]["value"]["adoptions"].as_object().unwrap();
        assert_eq!(adoptions.len(), 1);
        core.reply(&write["id"], json!({"version":1})).await;
    });
    app.on_action(A::NewWorkspace, 80);
    assert!(app.dialog.is_some(), "new_workspace prompts for the path");
    app.dialog_key(&press(Key::Enter));
    assert_eq!(app.pending_actions, 1);
    settle(&mut app).await;
    assert_eq!(app.pending_actions, 0);
    assert!(app.notice.is_none());
    server.await.unwrap();
}

#[tokio::test]
async fn the_mobile_new_workspace_target_suspends_the_switcher_and_dispatches() {
    let mut app = app_with_spaces(&[("t1", "work")]);
    pending(&mut app);
    app.focused = Some("t1".into());
    app.mode = Mode::Navigate;
    app.refresh_projection();
    let mut rx = operations(&mut app);
    let server = tokio::spawn(async move {
        let mut core = rx.recv().await.unwrap();
        core.read_hello().await;
        core.answer_hello(true, &["doc.get", "doc.set", "terminal.run"], &[])
            .await;
        let mut runs = 0;
        while runs == 0 {
            let req = core.read_request().await;
            if req["method"] == "terminal.run" {
                runs += 1;
                assert_eq!(req["params"]["keep"], true);
            }
            let result = match req["method"].as_str().unwrap() {
                "terminal.run" => testcore::minimal_terminal(),
                "doc.set" => json!({"version":1}),
                _ => json!({"value":null,"version":0}),
            };
            core.reply(&req["id"], result).await;
        }
    });
    app.on_mobile_target(&ClientMobileTarget::NewWorkspace);
    assert!(app.mobile_switcher_suspended);
    assert!(app.dialog.is_some(), "new_workspace prompts for the path");
    app.dialog_key(&press(Key::Enter));
    settle(&mut app).await;
    assert_eq!(app.pending_actions, 0);
    app.resume_mobile_switcher_if_ready();
    assert!(!app.mobile_switcher_suspended);
    server.await.unwrap();
}

#[tokio::test]
async fn escape_and_an_outside_press_close_the_menu() {
    let mut app = app_with_spaces(&[("t1", "work")]);
    pending(&mut app);
    app.hit_map.panes.push(PaneHit {
        rect: Rect::new(0, 0, 20, 10),
        inner_rect: Rect::new(0, 0, 20, 10),
        scrollbar_rect: None,
        scroll: None,
        pane_id: "t1".into(),
    });
    app.context_menu_press((2, 2));
    assert!(app.menu.is_some());
    app.menu_key(&press(Key::Escape));
    assert!(app.menu.is_none());
    app.context_menu_press((2, 2));
    assert!(app.menu.is_some());
    app.menu_mouse(input::MouseInput {
        kind: MouseEventKind::Down(MouseButton::Left),
        col: 60,
        row: 20,
        mods: Mods::NONE,
        sgr_position: None,
    });
    assert!(app.menu.is_none());
}

#[tokio::test]
async fn the_passthrough_toggle_round_trips_through_the_menu() {
    let mut app = app_with_spaces(&[("t1", "work")]);
    pending(&mut app);
    app.focused = Some("t1".into());
    app.refresh_projection();
    app.hit_map.panes.push(PaneHit {
        rect: Rect::new(0, 0, 20, 10),
        inner_rect: Rect::new(0, 0, 20, 10),
        scrollbar_rect: None,
        scroll: None,
        pane_id: "t1".into(),
    });
    app.context_menu_press((2, 2));
    let items = app.menu.as_ref().unwrap().items();
    let toggle = items
        .iter()
        .position(|i| i.action == ContextMenuAction::ToggleRightClickPassthrough)
        .unwrap();
    assert_eq!(items[toggle].label, "Send right-clicks to pane");
    for _ in 0..toggle {
        app.menu_key(&press(Key::ArrowDown));
    }
    app.menu_key(&press(Key::Enter));
    assert!(app.right_click_panes.contains("t1"));
    app.context_menu_press((2, 2));
    let items = app.menu.as_ref().unwrap().items();
    let toggle = items
        .iter()
        .position(|i| i.action == ContextMenuAction::ToggleRightClickPassthrough)
        .unwrap();
    assert_eq!(items[toggle].label, "Use Herdr right-click menu");
}

#[tokio::test]
async fn clicking_the_menu_launcher_opens_global_menu_and_dispatches_actions() {
    let mut app = app_with_spaces(&[("t1", "work")]);
    pending(&mut app);
    app.area = (120, 40);
    let backend = draw_to(&mut app, 120, 40);
    let launcher = app.hit_map.global_launcher;
    assert!(!launcher.is_empty(), "menu launcher hit rect is populated");
    assert_eq!(
        launcher.y, 39,
        "menu launcher sits in the bottom sidebar footer (Herdr.app parity)"
    );
    assert_eq!(
        app.hit_map.new_workspace.y, 1,
        "+ new sits at the top of the spaces section"
    );
    let lines = screen_lines(&backend);
    assert!(lines[38].starts_with("─"), "footer has a top border rule");
    assert!(lines[39].contains("menu"), "footer renders menu");
    assert!(lines[39].contains('«'), "footer renders collapse toggle");

    let left_click = |col, row| input::MouseInput {
        kind: MouseEventKind::Down(MouseButton::Left),
        col,
        row,
        mods: Mods::NONE,
        sgr_position: None,
    };

    // Click menu -> opens Global menu.
    app.on_mouse(left_click(launcher.x + 1, launcher.y));
    let Some(menu) = &app.menu else {
        panic!("clicking global_launcher opens the global menu");
    };
    assert!(matches!(menu.target, ContextMenuTarget::Global));
    let labels: Vec<_> = menu.items().iter().map(|i| i.label).collect();
    assert_eq!(
        labels,
        [
            "Settings",
            "Keybinds",
            "Machines",
            "Add machine",
            "Update",
            "Detach"
        ]
    );

    // Clicking menu again toggles it closed.
    app.on_mouse(left_click(launcher.x + 1, launcher.y));
    assert!(app.menu.is_none());

    // Open and activate Settings (index 0).
    app.on_mouse(left_click(launcher.x + 1, launcher.y));
    app.menu_key(&press(Key::Enter));
    assert!(app.settings.is_some());
    app.settings = None;

    // Open and activate Keybinds (index 1).
    app.on_mouse(left_click(launcher.x + 1, launcher.y));
    app.menu_key(&press(Key::ArrowDown));
    app.menu_key(&press(Key::Enter));
    assert!(app.help);
    app.help = false;

    // Open and activate Machines (index 2).
    app.on_mouse(left_click(launcher.x + 1, launcher.y));
    for _ in 0..2 {
        app.menu_key(&press(Key::ArrowDown));
    }
    app.menu_key(&press(Key::Enter));
    assert!(app.machines_modal.is_some());
    app.machines_modal = None;

    // Open and activate Add machine (index 3).
    app.on_mouse(left_click(launcher.x + 1, launcher.y));
    for _ in 0..3 {
        app.menu_key(&press(Key::ArrowDown));
    }
    app.menu_key(&press(Key::Enter));
    assert!(matches!(
        app.dialog.as_ref().map(|d| &d.target),
        Some(super::super::dialogs::Target::MachineAdd)
    ));
    app.dialog = None;

    // Open and activate Update (index 4).
    app.update_latest = Some("9.9.9".into());
    app.on_mouse(left_click(launcher.x + 1, launcher.y));
    for _ in 0..4 {
        app.menu_key(&press(Key::ArrowDown));
    }
    app.menu_key(&press(Key::Enter));
    assert!(app.update_modal.is_some());
    app.update_modal = None;

    // Open and activate Detach (index 5).
    app.on_mouse(left_click(launcher.x + 1, launcher.y));
    for _ in 0..5 {
        app.menu_key(&press(Key::ArrowDown));
    }
    app.menu_key(&press(Key::Enter));
    assert!(app.quitting);
}

#[tokio::test]
async fn closing_a_workspace_on_a_non_active_machine_resolves_membership_and_keeps_focus() {
    use super::endpoint_nav::{app_multi, spaced_term};
    use crate::sidebar::fixture::machine;

    let mut app = app_multi(
        vec![
            spaced_term("t1", "local-ws"),
            spaced_term("host-a/t2", "remote-ws"),
        ],
        vec![machine("host-a", "connected")],
    );
    pending(&mut app);
    app.focused = Some("t1".into());
    app.shell.confirm_close = false;
    app.refresh_projection();
    app.area = (120, 40);
    draw_to(&mut app, 120, 40);

    // Right-click the non-active remote workspace row while Local is focused.
    let remote_hit = app
        .hit_map
        .workspaces
        .iter()
        .find(|h| h.endpoint_id == ClientEndpointId::Remote("host-a".into()))
        .expect("remote workspace hit")
        .clone();
    app.context_menu_press((remote_hit.rect.x, remote_hit.rect.y));
    let Some(menu) = &app.menu else {
        panic!("right-clicking a non-active endpoint workspace opens its menu");
    };
    assert!(matches!(
        &menu.target,
        ContextMenuTarget::Workspace { endpoint_id, workspace_id, .. }
            if *endpoint_id == ClientEndpointId::Remote("host-a".into())
                && workspace_id == &remote_hit.workspace_id
    ));

    let created_ms = app.view.inventory.get("host-a/t2").unwrap().created_ms;
    let mut rx = operations(&mut app);
    let server = tokio::spawn(async move {
        let mut core = rx.recv().await.unwrap();
        core.read_hello().await;
        core.answer_hello(
            true,
            &[
                "doc.get",
                "doc.set",
                "terminal.get",
                "terminal.kill",
                "terminal.rm",
            ],
            &[],
        )
        .await;
        // Initial doc.get for hc-tui validation
        let doc = core.read_request().await;
        assert_eq!(doc["method"], "doc.get");
        core.reply(&doc["id"], json!({"value":null,"version":0}))
            .await;
        // terminal.get for incarnation verification on host-a/t2
        let get = core.read_request().await;
        assert_eq!(get["method"], "terminal.get");
        assert_eq!(get["params"]["terminal"], "host-a/t2");
        let mut t = testcore::minimal_terminal();
        t["address"] = json!("host-a/t2");
        t["created_ms"] = json!(created_ms);
        t["labels"] = json!({"herdr.space": "remote-ws", "herdr.tab": "tab"});
        core.reply(&get["id"], t.clone()).await;
        // doc.get herdr-app on host-a
        let foreign = core.read_request().await;
        assert_eq!(foreign["method"], "doc.get");
        assert_eq!(foreign["params"]["on"], "host-a");
        core.reply(&foreign["id"], json!({"value":null,"version":0}))
            .await;
        // terminal.kill on host-a/t2
        let kill = core.read_request().await;
        assert_eq!(kill["method"], "terminal.kill");
        assert_eq!(kill["params"]["terminal"], "host-a/t2");
        core.reply(&kill["id"], t).await;
        // terminal.rm on host-a/t2
        let rm = core.read_request().await;
        assert_eq!(rm["method"], "terminal.rm");
        core.reply(&rm["id"], json!({})).await;
        // doc.get + doc.set to hide the closed terminal
        let get_doc = core.read_request().await;
        core.reply(&get_doc["id"], json!({"value":null,"version":0}))
            .await;
        let set_doc = core.read_request().await;
        assert_eq!(set_doc["method"], "doc.set");
        core.reply(&set_doc["id"], json!({"version":1})).await;
    });

    // Item 1 is "Close".
    app.menu_key(&press(Key::ArrowDown));
    app.menu_key(&press(Key::Enter));
    assert_eq!(app.pending_actions, 1);
    settle(&mut app).await;
    assert_eq!(app.pending_actions, 0);
    assert!(
        app.notice.is_none(),
        "closing remote workspace should not show an error notice"
    );
    assert_eq!(
        app.focused.as_deref(),
        Some("t1"),
        "closing remote workspace keeps Local focus"
    );
    server.await.unwrap();
}

#[tokio::test]
async fn new_workspace_accepts_custom_path_and_works_when_empty() {
    // Empty shell: no terminals, no workspaces, no focused pane.
    let mut app = super::endpoint_nav::app_multi(vec![], vec![]);
    pending(&mut app);
    app.focused = None;
    app.refresh_projection();
    let mut rx = operations(&mut app);
    let server = tokio::spawn(async move {
        let mut core = rx.recv().await.unwrap();
        core.read_hello().await;
        core.answer_hello(true, &["doc.get", "doc.set", "terminal.run"], &[])
            .await;
        for _ in 0..2 {
            let read = core.read_request().await;
            assert_eq!(read["method"], "doc.get");
            core.reply(&read["id"], json!({"value":null,"version":0}))
                .await;
        }
        let run = core.read_request().await;
        assert_eq!(run["method"], "terminal.run");
        assert_eq!(run["params"]["cwd"], "/home/user/acme-shop");
        let mut terminal = testcore::minimal_terminal();
        terminal["address"] = json!("t1");
        terminal["cwd"] = json!("/home/user/acme-shop");
        core.reply(&run["id"], terminal).await;
        let read = core.read_request().await;
        core.reply(&read["id"], json!({"value":null,"version":0}))
            .await;
        let write = core.read_request().await;
        assert_eq!(write["method"], "doc.set");
        let adoptions = write["params"]["value"]["adoptions"].as_object().unwrap();
        assert_eq!(adoptions.len(), 1);
        let adoption = adoptions.values().next().unwrap();
        assert_eq!(adoption["name"], "acme-shop");
        core.reply(&write["id"], json!({"version":1})).await;
    });

    app.on_action(A::NewWorkspace, 80);
    assert!(app.dialog.is_some());
    clear_rename(&mut app);
    app.insert_dialog_text("/home/user/acme-shop");
    app.dialog_key(&press(Key::Enter));
    assert_eq!(app.pending_actions, 1);
    settle(&mut app).await;
    assert_eq!(app.pending_actions, 0);
    assert!(app.notice.is_none());
    server.await.unwrap();
}

#[tokio::test]
async fn inline_machine_plus_creates_workspace_on_non_active_machine_with_space_name_label() {
    use super::endpoint_nav::{app_multi, spaced_term};
    use crate::sidebar::fixture::machine;

    let mut app = app_multi(
        vec![spaced_term("t1", "local-ws")],
        vec![machine("host-a", "connected")],
    );
    pending(&mut app);
    app.focused = Some("t1".into());
    app.refresh_projection();
    app.area = (120, 40);
    draw_to(&mut app, 120, 40);

    let remote_hit = app
        .hit_map
        .machines
        .iter()
        .find(|h| h.endpoint_id == ClientEndpointId::Remote("host-a".into()))
        .expect("remote machine hit")
        .clone();
    assert!(
        !remote_hit.new_workspace.is_empty(),
        "online remote machine row exposes inline + hit"
    );

    let mut rx = operations(&mut app);
    let server = tokio::spawn(async move {
        let mut core = rx.recv().await.unwrap();
        core.read_hello().await;
        core.answer_hello(true, &["doc.get", "doc.set", "terminal.run"], &[])
            .await;
        for _ in 0..2 {
            let read = core.read_request().await;
            core.reply(&read["id"], json!({"value":null,"version":0}))
                .await;
        }
        let run = core.read_request().await;
        assert_eq!(run["method"], "terminal.run");
        assert_eq!(run["params"]["on"], "host-a");
        assert_eq!(run["params"]["cwd"], "/srv/api");
        assert_eq!(run["params"]["labels"]["herdr.space.name"], "api");
        let mut terminal = testcore::minimal_terminal();
        terminal["address"] = json!("host-a/t2");
        terminal["cwd"] = json!("/srv/api");
        core.reply(&run["id"], terminal).await;
        let read = core.read_request().await;
        core.reply(&read["id"], json!({"value":null,"version":0}))
            .await;
        let write = core.read_request().await;
        core.reply(&write["id"], json!({"version":1})).await;
    });

    app.on_mouse(input::MouseInput {
        kind: MouseEventKind::Down(MouseButton::Left),
        col: remote_hit.new_workspace.x,
        row: remote_hit.new_workspace.y,
        mods: Mods::NONE,
        sgr_position: None,
    });
    assert!(app.dialog.is_some());
    app.insert_dialog_text("/srv/api");
    app.dialog_key(&press(Key::Enter));
    assert_eq!(app.pending_actions, 1);
    settle(&mut app).await;
    assert_eq!(app.pending_actions, 0);
    assert!(app.notice.is_none());
    server.await.unwrap();
}

#[test]
fn ssh_config_parser_discovers_concrete_hosts_and_skips_wildcards() {
    let parsed = super::super::machines_modal::parse_ssh_config_text(
        "# comment\nHost *\n  User ignored\n\nHost host-a alias-a\n  HostName 10.0.0.1\n  User alice\n\nHost host-b\n  HostName host-b.internal\n",
    );
    assert_eq!(
        parsed,
        vec![
            ("host-a".into(), "alice@10.0.0.1".into()),
            ("host-b".into(), "host-b.internal".into()),
        ]
    );
}

#[tokio::test]
async fn machines_modal_renders_and_adds_discovered_ssh_host() {
    use super::endpoint_nav::{app_multi, spaced_term};
    use crate::sidebar::fixture::machine;

    let mut app = app_multi(
        vec![spaced_term("t1", "local-ws")],
        vec![machine("host-a", "unreachable")],
    );
    pending(&mut app);
    app.area = (100, 30);
    app.machines_modal = Some(super::super::machines_modal::MachinesModal {
        ssh_hosts: vec![
            ("host-a".into(), "alice@10.0.0.1".into()),
            ("host-b".into(), "alice@10.0.0.2".into()),
        ],
        selected: 0,
        render: Default::default(),
    });
    let backend = draw_to(&mut app, 100, 30);
    let screen = screen_lines(&backend).join("\n");
    assert!(screen.contains("machines"), "title renders");
    assert!(screen.contains("host-a"), "configured machine renders");
    assert!(screen.contains("host-b"), "discovered ssh host renders");

    let mut rx = operations(&mut app);
    let server = tokio::spawn(async move {
        let mut core = rx.recv().await.unwrap();
        core.read_hello().await;
        core.answer_hello(true, &["machine.add"], &[]).await;
        let req = core.read_request().await;
        assert_eq!(req["method"], "machine.add");
        assert_eq!(req["params"], json!({"target": "host-b"}));
        core.reply(&req["id"], json!({})).await;
    });

    app.handle_machines_key(&press(Key::ArrowDown));
    app.handle_machines_key(&press(Key::Enter));
    assert!(app.machines_modal.is_none());
    assert_eq!(app.pending_actions, 1);
    settle(&mut app).await;
    assert_eq!(app.pending_actions, 0);
    assert!(app.notice.is_none());
    server.await.unwrap();
}

#[tokio::test]
async fn clicking_offline_machine_status_badge_triggers_reconnect() {
    use super::endpoint_nav::{app_multi, spaced_term};
    use crate::sidebar::fixture::machine;

    let mut app = app_multi(
        vec![spaced_term("t1", "local-ws")],
        vec![machine("host-a", "unreachable")],
    );
    pending(&mut app);
    app.focused = Some("t1".into());
    app.refresh_projection();
    app.area = (120, 40);
    draw_to(&mut app, 120, 40);

    let remote_hit = app
        .hit_map
        .machines
        .iter()
        .find(|h| h.endpoint_id == ClientEndpointId::Remote("host-a".into()))
        .expect("remote machine hit")
        .clone();
    assert!(!remote_hit.status_badge.is_empty());

    let mut rx = operations(&mut app);
    let server = tokio::spawn(async move {
        let mut core = rx.recv().await.unwrap();
        core.read_hello().await;
        core.answer_hello(true, &["machine.connect"], &[]).await;
        let req = core.read_request().await;
        assert_eq!(req["method"], "machine.connect");
        assert_eq!(req["params"], json!({"machine": "host-a"}));
        core.reply(&req["id"], json!({})).await;
    });

    app.on_mouse(input::MouseInput {
        kind: MouseEventKind::Down(MouseButton::Left),
        col: remote_hit.status_badge.x,
        row: remote_hit.status_badge.y,
        mods: Mods::NONE,
        sgr_position: None,
    });
    assert_eq!(app.pending_actions, 1);
    settle(&mut app).await;
    assert_eq!(app.pending_actions, 0);
    server.await.unwrap();
}
