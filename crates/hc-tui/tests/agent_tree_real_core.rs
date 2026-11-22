//! Opt-in worker-tree smoke against a source-built core, in owned scratch state.
#![cfg(unix)]

#[path = "common/pty_tui.rs"]
mod pty;
#[path = "../../hc-proto/tests/common/scratch.rs"]
mod scratch;

use hc_proto::{client::Client, endpoint::Endpoint, terminal::RunParams};
use scratch::ScratchCore;
use std::time::Instant;

async fn reporter(
    core: &ScratchCore,
    client: &Client,
    name: &str,
    state: &str,
    parent: Option<String>,
) -> String {
    client.terminal_run(RunParams {
        name: Some(name.into()),
        labels: [("herdr.tab".into(), name.into())].into(),
        parent,
        cwd: Some(core.work.to_string_lossy().into_owned()),
        argv: vec!["sh".into(), "-c".into(), format!("printf '\\033]7501;state={state}:app={name}\\033\\\\'; while read line; do :; done")],
        ..Default::default()
    }).await.expect("create reporting terminal").terminal.address
}

#[tokio::test]
#[ignore = "needs HC_TUI_TEST_CORE: source-built core in an isolated scratch session"]
async fn real_core_worker_tree_and_clean_detach() {
    let core = ScratchCore::spawn().await;
    let client = Client::connect(&core.endpoint, None::<&std::ffi::OsStr>)
        .await
        .unwrap();
    let boss = reporter(&core, &client, "treeboss", "idle", None).await;
    let worker = reporter(&core, &client, "treeworker", "blocked", Some(boss)).await;
    reporter(&core, &client, "treesolo", "working", None).await;
    let Endpoint::Local(local) = &core.endpoint else {
        panic!("local scratch");
    };
    let mut env = pty::env_base(&core.root, &core.session, &local.socket);
    env.push(("HC_TUI_NO_UPDATE_CHECK".into(), "1".into()));
    let mut tui = pty::spawn_tui(&env, &["--agent-tree".into()], &core.work);
    let ready = pty::until(
        &mut tui,
        |vt, _| {
            let text = pty::screen(vt);
            let sidebar: Vec<String> = text.lines().map(|l| l.chars().take(26).collect()).collect();
            let Some(header) = sidebar.iter().position(|l| l.contains("agents")) else {
                return false;
            };
            let rows = &sidebar[header + 1..];
            let positions: Vec<_> = ["treeboss", "treeworker", "treesolo"]
                .iter()
                .map(|name| rows.iter().position(|l| l.contains(name)))
                .collect();
            matches!(positions.as_slice(), [Some(a), Some(b), Some(c)] if a < b && b < c)
                && rows.iter().any(|l| l.contains("└─"))
        },
        "real core tree",
    )
    .await;
    assert!(ready, "tree not rendered:\n{}", tui.screen());
    let renamed = std::process::Command::new(scratch::core_binary())
        .env_clear()
        .envs(env.iter().cloned())
        .args(["label", &worker, "herdr.tab.name=worker done"])
        .output()
        .expect("set delegate title on scratch core");
    assert!(renamed.status.success(), "{renamed:?}");
    assert!(
        pty::until(
            &mut tui,
            |vt, _| pty::screen(vt).contains("worker done"),
            "updated delegate tab title",
        )
        .await,
        "delegate title not rendered:\n{}",
        tui.screen(),
    );
    tui.type_keys(b"\x02q");
    let deadline = Instant::now() + pty::DEADLINE;
    loop {
        tui.pump();
        if let Some(status) = tui.child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline, "detach timed out");
        tokio::time::sleep(pty::POLL).await;
    }
    drop(tui);
    drop(client);
    core.finish().expect("verified scratch cleanup");
}
