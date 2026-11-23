//! `terminal.list`'s `machines`: an empty list is an answer.
//!
//! Core 0.1.21 sends `machines` always (`[]` without machines), so the
//! static fallback records that answer instead of leaving the machine
//! facts unknown. Older cores omit the member when empty; their `[]`
//! reads the same, which is also the truth for them.

use super::*;
use crate::terminal::ListResult;
use serde_json::json;

fn actor() -> Actor {
    let (version, _) = watch::channel(0);
    let (_, commands) = mpsc::channel(1);
    let (_, stop) = watch::channel(false);
    Actor::new(
        WatchParams::default(),
        Arc::new(Mutex::new(View::default())),
        version,
        commands,
        stop,
        Arc::new(|| Box::pin(async { panic!("no RPC transport is opened") })),
        Arc::new(|_| Box::pin(async { panic!("no subscription transport is opened") })),
    )
}

fn list(machines: serde_json::Value) -> ListResult {
    serde_json::from_value(json!({
        "terminals": [],
        "hidden_finished": 0,
        "machines": machines,
    }))
    .unwrap()
}

#[test]
fn an_empty_machines_list_is_the_answer_not_missing_data() {
    let mut actor = actor();
    assert!(actor.machines.is_none(), "unknown before any list");
    actor.static_done(Ok((0, Ok(list(json!([]))))));
    assert_eq!(
        actor.machines.as_deref(),
        Some(&[][..]),
        "an empty list means this core has no machines"
    );
}

#[test]
fn an_older_core_that_omits_machines_reads_the_same() {
    let mut actor = actor();
    let omitted: ListResult = serde_json::from_value(json!({
        "terminals": [],
        "hidden_finished": 0,
    }))
    .unwrap();
    actor.static_done(Ok((0, Ok(omitted))));
    assert_eq!(actor.machines.as_deref(), Some(&[][..]));
}

#[test]
fn reconnecting_machine_refetches_watched_documents() {
    use crate::machine::{MachineListResult, MachineView};
    use crate::watch::DocKey;

    let mut actor = actor();
    actor.params.documents = vec!["herdr-app".into()];
    let mut m = MachineView {
        name: "host-a".into(),
        target: "alice@host-a".into(),
        id: "id-a".into(),
        host: "host-a".into(),
        hostname: None,
        version: "0.1.22".into(),
        state: "unreachable".into(),
        session: None,
        since_ms: None,
        temporary: false,
        via: false,
        why: Some("offline".into()),
    };
    let epoch = actor.rpc_epoch;
    actor.machines_done(Ok((
        epoch,
        Ok(MachineListResult {
            machines: vec![m.clone()],
        }),
    )));
    // Consume the initial doc request while unreachable.
    actor.doc_pending.clear();

    // Machine transitions to connected -> must refetch herdr-app doc.
    m.state = "connected".into();
    m.why = None;
    actor.machines_done(Ok((epoch, Ok(MachineListResult { machines: vec![m] }))));
    assert!(
        actor.doc_pending.contains(&DocKey {
            machine: Some("host-a".into()),
            name: "herdr-app".into(),
        }),
        "transitioning a machine to connected must re-fetch its herdr-app document"
    );
}
