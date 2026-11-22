//! Unix-socket fake core for `agent_tree_e2e.rs`.
//!
//! Speaks official-core methods only (`features: []`) over a scratch Unix
//! socket, records every JSON-RPC `(method, params)` call, and aborts all
//! background tasks on drop.

#![cfg(unix)]
#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use hc_proto::frame::{Frame, Kind};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::watch;

use crate::fakecore;

pub const METHODS: &[&str] = &[
    "hello",
    "terminal.list",
    "terminal.attach",
    "terminal.label",
    "machine.list",
    "doc.get",
    "doc.set",
    "events.subscribe",
];

pub const FORBIDDEN_MUTATIONS: &[&str] = &[
    "terminal.run",
    "terminal.kill",
    "terminal.send",
    "terminal.interrupt",
    "terminal.keys",
    "terminal.keep",
    "machine.add",
    "machine.remove",
    "machine.rename",
];

#[cfg(target_os = "macos")]
pub fn scratch_parent() -> PathBuf {
    PathBuf::from("/tmp")
}

#[cfg(not(target_os = "macos"))]
pub fn scratch_parent() -> PathBuf {
    std::env::temp_dir()
}

pub fn fixture_terminals() -> Vec<Value> {
    vec![
        json!({
            "address": "t1",
            "name": "boss",
            "labels": {"herdr.space": "alpha", "herdr.tab": "t1"},
            "argv": ["sh"],
            "cwd": "/home/user/acme-shop",
            "state": "live",
            "pid": 101,
            "created_ms": 1,
            "screen": "exact",
            "cols": 80,
            "rows": 24,
            "reported": true,
            "status": {"state": "idle"},
            "turn": 0,
        }),
        json!({
            "address": "t2",
            "name": "mid-shell",
            "labels": {"herdr.space": "alpha", "herdr.tab": "t2"},
            "argv": ["sh"],
            "cwd": "/home/user/acme-shop",
            "state": "live",
            "pid": 102,
            "created_ms": 2,
            "screen": "exact",
            "cols": 80,
            "rows": 24,
            "reported": false,
            "parent": "t1",
            "turn": 0,
        }),
        json!({
            "address": "host-b/t3",
            "name": "worker",
            "labels": {"herdr.space": "beta", "herdr.tab": "t3"},
            "argv": ["sh"],
            "cwd": "/home/user/acme-shop",
            "state": "live",
            "pid": 103,
            "created_ms": 3,
            "screen": "exact",
            "cols": 80,
            "rows": 24,
            "reported": true,
            "status": {"state": "blocked"},
            "parent": "t2",
            "turn": 0,
        }),
        json!({
            "address": "t4",
            "name": "solo",
            "labels": {"herdr.space": "gamma", "herdr.tab": "t4"},
            "argv": ["sh"],
            "cwd": "/home/user/acme-shop",
            "state": "live",
            "pid": 104,
            "created_ms": 4,
            "screen": "exact",
            "cols": 80,
            "rows": 24,
            "reported": true,
            "status": {"state": "working"},
            "turn": 0,
        }),
    ]
}

fn terminal_by_address(address: &str) -> Value {
    fixture_terminals()
        .into_iter()
        .find(|term| term["address"] == address)
        .unwrap_or_else(|| {
            json!({
                "address": address,
                "labels": {},
                "argv": ["sh"],
                "cwd": "/home/user/acme-shop",
                "state": "live",
                "pid": 199,
                "created_ms": 99,
                "screen": "exact",
                "cols": 80,
                "rows": 24,
            })
        })
}

pub struct FakeServer {
    socket: PathBuf,
    calls: Arc<Mutex<Vec<(String, Value)>>>,
    shutdown: watch::Sender<bool>,
    accept_task: tokio::task::AbortHandle,
    conn_tasks: Arc<Mutex<Vec<tokio::task::AbortHandle>>>,
}

impl FakeServer {
    pub fn start(socket: PathBuf) -> Self {
        let listener = UnixListener::bind(&socket).expect("bind fake core socket");
        let calls = Arc::new(Mutex::new(Vec::new()));
        let (shutdown, shutdown_rx) = watch::channel(false);
        let conn_tasks: Arc<Mutex<Vec<tokio::task::AbortHandle>>> =
            Arc::new(Mutex::new(Vec::new()));
        let next_channel = Arc::new(AtomicU32::new(1));

        let accept_calls = Arc::clone(&calls);
        let accept_conns = Arc::clone(&conn_tasks);
        let accept_handle = tokio::spawn(async move {
            let mut rx = shutdown_rx;
            loop {
                tokio::select! {
                    _ = rx.changed() => break,
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else { break };
                        let conn_calls = Arc::clone(&accept_calls);
                        let conn_rx = rx.clone();
                        let channels = Arc::clone(&next_channel);
                        let handle = tokio::spawn(serve_conn(stream, conn_calls, conn_rx, channels));
                        accept_conns.lock().unwrap().push(handle.abort_handle());
                    }
                }
            }
        });

        Self {
            socket,
            calls,
            shutdown,
            accept_task: accept_handle.abort_handle(),
            conn_tasks,
        }
    }

    pub fn calls(&self) -> Vec<(String, Value)> {
        self.calls.lock().unwrap().clone()
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        self.accept_task.abort();
        for task in self.conn_tasks.lock().unwrap().drain(..) {
            task.abort();
        }
        let _ = fs::remove_file(&self.socket);
    }
}

async fn serve_conn(
    stream: UnixStream,
    calls: Arc<Mutex<Vec<(String, Value)>>>,
    mut shutdown: watch::Receiver<bool>,
    next_channel: Arc<AtomicU32>,
) {
    let (read, write) = stream.into_split();
    let mut core = fakecore::FakeCore::new(read, write);
    let hello = tokio::select! {
        _ = shutdown.changed() => return,
        req = core.read_hello() => req,
    };
    if hello["params"]["framed"] != true {
        core.answer_hello(fakecore::hello_result_framed(METHODS, &[], false))
            .await;
        let sub = tokio::select! {
            _ = shutdown.changed() => return,
            req = core.read_message() => req,
        };
        let method = sub["method"].as_str().unwrap_or("").to_owned();
        calls.lock().unwrap().push((method, sub["params"].clone()));
        core.reply_line(
            &sub["id"],
            json!({"seq": 1, "terminals": fixture_terminals()}),
        )
        .await;
        let _ = shutdown.changed().await;
        return;
    }

    core.answer_hello(fakecore::hello_result_features(METHODS, &[]))
        .await;
    let (mut reader, mut out) = core.into_parts();
    loop {
        let frame = tokio::select! {
            _ = shutdown.changed() => break,
            next = reader.read_frame() => match next {
                Ok(Some(frame)) => frame,
                Ok(None) | Err(_) => break,
            },
        };
        if frame.channel > 0 && frame.kind == Kind::Close {
            let ack = Frame::close(frame.channel, Vec::new()).encode();
            if out.write_all(&ack).await.is_err() {
                break;
            }
            continue;
        }
        if frame.channel != 0 || frame.kind != Kind::Data {
            continue;
        }
        let Ok(req) = serde_json::from_slice::<Value>(&frame.payload) else {
            break;
        };
        let method = req["method"].as_str().unwrap_or("").to_owned();
        let params = req["params"].clone();
        calls.lock().unwrap().push((method.clone(), params.clone()));
        let (result, banner) = match method.as_str() {
            "machine.list" => (
                json!({
                    "machines": [{
                        "name": "host-b",
                        "target": "host-b-ssh",
                        "id": "id-host-b",
                        "host": "host-b",
                        "version": "0.1.21",
                        "state": "connected",
                    }]
                }),
                None,
            ),
            "doc.get" => (json!({"value": null, "version": 0}), None),
            "doc.set" => (json!({"version": 1}), None),
            "terminal.list" => (json!({"terminals": fixture_terminals()}), None),
            "terminal.attach" => {
                let target = params["terminal"].as_str().unwrap_or("t1").to_owned();
                let channel = next_channel.fetch_add(1, Ordering::Relaxed);
                let result =
                    fakecore::attach_result(channel, terminal_by_address(&target), true, false);
                (result, Some((channel, format!("PANE {target}\r\n"))))
            }
            _ => (json!({}), None),
        };
        let payload =
            serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": req["id"], "result": result}))
                .unwrap();
        if out
            .write_all(&Frame::data(0, payload).encode())
            .await
            .is_err()
        {
            break;
        }
        if let Some((channel, text)) = banner {
            let frame = Frame::new(channel, Kind::Data, text.into_bytes()).encode();
            if out.write_all(&frame).await.is_err() {
                break;
            }
        }
    }
}
