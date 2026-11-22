//! Isolated PTY + fake-core e2e verification for `--agent-tree` vs default flat mode.
//!
//! Spawns the compiled `hc-tui` binary inside a private PTY against a scratch
//! Unix-socket fake core with `env_clear()` and a scratch Herdr config. Never
//! connects to real `hc` sessions, never touches real user config, and uses
//! official-core methods only.

#![cfg(unix)]

#[path = "common/agent_tree_core.rs"]
mod core_fake;
#[path = "../../hc-proto/tests/common/fakecore.rs"]
mod fakecore;
#[path = "common/pty_tui.rs"]
mod pty;
mod support;

use std::fs;
use std::path::Path;
use std::time::Instant;

use serde_json::Value;

use core_fake::{FORBIDDEN_MUTATIONS, FakeServer, scratch_parent};
use support::TestDir;

fn write_config(config_home: &Path, prefix: &str) {
    let dir = config_home.join("herdr");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("config.toml"),
        format!(
            "[ui]\nagent_panel_sort = \"priority\"\n\n\
             [keys]\nprefix = \"{prefix}\"\n\
             previous_agent = \"alt+up\"\n\
             next_agent = \"alt+down\"\n"
        ),
    )
    .unwrap();
}

fn host_b_online(screen: &str) -> bool {
    let sidebar: Vec<String> = screen
        .lines()
        .map(|line| line.chars().take(26).collect())
        .collect();
    let end = sidebar
        .iter()
        .position(|line| line.contains("agents"))
        .unwrap_or(sidebar.len());
    sidebar[..end]
        .iter()
        .any(|line| line.contains("host-b") && !line.contains('!'))
}

fn agents_section(screen: &str) -> Vec<String> {
    let sidebar: Vec<String> = screen
        .lines()
        .map(|line| line.chars().take(26).collect())
        .collect();
    let Some(header) = sidebar.iter().position(|line| line.contains("agents")) else {
        return Vec::new();
    };
    sidebar[header + 1..].to_vec()
}

fn row_of(lines: &[String], needle: &str) -> Option<usize> {
    lines.iter().position(|line| line.contains(needle))
}

fn assert_tree_layout(screen: &str, context: &str) {
    let section = agents_section(screen);
    assert!(
        row_of(&section, "mid-shell").is_none(),
        "non-agent shell must not appear in Agents ({context}):\n{screen}"
    );
    let y_boss = row_of(&section, "boss").expect("boss row");
    let y_worker = row_of(&section, "worker").expect("worker row");
    let y_solo = row_of(&section, "solo").expect("solo row");
    assert!(
        y_boss < y_worker && y_worker < y_solo,
        "tree order should lift boss ahead of blocked worker and solo ({context}):\n{screen}"
    );
    assert!(
        section[y_boss..=y_worker]
            .iter()
            .any(|line| line.contains("└─")),
        "tree mode should draw branch connector for nested worker ({context}):\n{screen}"
    );
}

async fn detach_with(tui: &mut pty::Tui, chord: &[u8]) {
    tui.type_keys(chord);
    let deadline = Instant::now() + pty::DEADLINE;
    let status = loop {
        tui.pump();
        match tui.child.try_wait().expect("try_wait") {
            Some(status) => break status,
            None => {
                assert!(
                    Instant::now() < deadline,
                    "hc-tui did not exit after detach; screen was:\n{}",
                    tui.screen()
                );
                tokio::time::sleep(pty::POLL).await;
            }
        }
    };
    tui.pump();
    let tail_start = tui.raw.len().saturating_sub(2048);
    let raw_tail = String::from_utf8_lossy(&tui.raw[tail_start..]).into_owned();
    assert!(
        status.success(),
        "hc-tui exited with {status}\nscreen:\n{}\nraw tail:\n{raw_tail}",
        tui.screen()
    );
}

async fn run_mode(tree: bool) -> Vec<(String, Value)> {
    let root = TestDir::new_in(&scratch_parent(), "hctui-tree");
    for dir in ["home", "state", "config", "run"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    write_config(&root.join("config"), "ctrl+f");
    let socket = root.join("core.sock");
    let server = FakeServer::start(socket.clone());

    let mut env = pty::env_base(&root, "hctui-tree-e2e", &socket);
    env.push(("HC_TUI_NO_UPDATE_CHECK".into(), "1".into()));
    let args = if tree {
        vec!["--agent-tree".into()]
    } else {
        Vec::new()
    };
    let mut tui = pty::spawn_tui(&env, &args, &root);

    let ready = pty::until(
        &mut tui,
        |vt, _| {
            let full = pty::screen(vt);
            let section = agents_section(&full);
            host_b_online(&full)
                && full.contains("PANE ")
                && row_of(&section, "boss").is_some()
                && row_of(&section, "worker").is_some()
                && row_of(&section, "solo").is_some()
        },
        "agents panel and online remote machine render fixture rows",
    )
    .await;
    let screen = tui.screen();
    assert!(
        ready,
        "agents panel never settled (calls={:?}):\n{screen}",
        server.calls()
    );

    if tree {
        assert_tree_layout(&screen, "initial");

        // Prove reload actually ran: change prefix to ctrl+a, reload via old
        // ctrl+f Shift+R, then open and close help using the new ctrl+a prefix.
        write_config(&root.join("config"), "ctrl+a");
        tui.type_keys(b"\x06R");
        tui.type_keys(b"\x01?");
        let help_opened = pty::until(
            &mut tui,
            |vt, _| pty::screen(vt).contains("close esc/enter"),
            "help overlay opened via reloaded ctrl+a prefix",
        )
        .await;
        assert!(
            help_opened,
            "reloaded ctrl+a ? did not open help:\n{}",
            tui.screen()
        );
        tui.type_keys(b"\x1b");
        let help_closed = pty::until(
            &mut tui,
            |vt, _| !pty::screen(vt).contains("close esc/enter"),
            "help overlay closed",
        )
        .await;
        assert!(help_closed, "help overlay did not close:\n{}", tui.screen());
        assert_tree_layout(&tui.screen(), "after reload");
        detach_with(&mut tui, b"\x01q").await;
    } else {
        let section = agents_section(&screen);
        assert!(
            row_of(&section, "mid-shell").is_none(),
            "non-agent shell must not appear in Agents (flat):\n{screen}"
        );
        let y_boss = row_of(&section, "boss").expect("boss row");
        let y_worker = row_of(&section, "worker").expect("worker row");
        let y_solo = row_of(&section, "solo").expect("solo row");
        assert!(
            y_worker < y_solo && y_solo < y_boss,
            "flat priority order should be worker < solo < boss:\n{screen}"
        );
        assert!(
            !section.iter().any(|line| line.contains("└─")),
            "flat mode must not draw tree connectors:\n{screen}"
        );
        detach_with(&mut tui, b"\x06q").await;
    }

    let calls = server.calls();
    drop(tui);
    drop(server);
    calls
}

#[tokio::test]
async fn cli_agent_tree_shows_family_order_and_preserves_flat_mode() {
    for tree in [false, true] {
        let calls = run_mode(tree).await;
        for (method, params) in &calls {
            assert!(
                !FORBIDDEN_MUTATIONS.contains(&method.as_str()),
                "unexpected mutating RPC {method} in tree={tree}: {params}"
            );
            if method == "doc.set" {
                assert_ne!(
                    params["name"], "herdr-app",
                    "must never write herdr-app document"
                );
            }
        }
    }
}
