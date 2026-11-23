//! TUI-native Machines overlay state, `~/.ssh/config` host discovery, and
//! keyboard/mouse routing.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crossterm::event::KeyCode;
use ratatui::layout::Position;

use super::input::{MouseButton, MouseEventKind, MouseInput, TerminalKey};
use super::*;
use crate::chrome::machines_modal::{MachineEntry, MachinesButton, MachinesModalRender};
use crate::operations::{Action, MachineOp};

pub(super) struct MachinesModal {
    pub ssh_hosts: Vec<(String, String)>,
    pub selected: usize,
    pub render: MachinesModalRender,
}

impl App {
    pub(super) fn open_machines_modal(&mut self) {
        self.release_pane_mouse();
        self.chrome_drag = None;
        self.watcher.refresh();
        let ssh_hosts = discover_ssh_hosts();
        self.machines_modal = Some(MachinesModal {
            ssh_hosts,
            selected: 0,
            render: MachinesModalRender::default(),
        });
    }

    pub(super) fn machines_modal_entries(&self) -> Vec<MachineEntry> {
        let mut entries = Vec::new();
        let mut known = HashSet::new();
        if let Some(machines) = self.view.machines.as_deref() {
            for m in machines.iter().filter(|m| !m.temporary) {
                known.insert(m.name.clone());
                known.insert(m.target.clone());
                known.insert(m.host.clone());
                entries.push(MachineEntry::Configured {
                    name: m.name.clone(),
                    target: m.target.clone(),
                    version: m.version.clone(),
                    state: m.state.clone(),
                    why: m.why.clone(),
                });
            }
        }
        if let Some(modal) = &self.machines_modal {
            for (alias, detail) in &modal.ssh_hosts {
                if !known.contains(alias) {
                    entries.push(MachineEntry::SshHost {
                        alias: alias.clone(),
                        detail: detail.clone(),
                    });
                }
            }
        }
        entries
    }

    pub(super) fn handle_machines_key(&mut self, key: &TerminalKey) {
        let combo = crate::input::terminal_key_of(key);
        let entries = self.machines_modal_entries();
        let Some(modal) = self.machines_modal.as_mut() else {
            return;
        };
        if !entries.is_empty() {
            modal.selected = modal.selected.min(entries.len() - 1);
        } else {
            modal.selected = 0;
        }
        match combo.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.machines_modal = None;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                modal.selected = modal.selected.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !entries.is_empty() {
                    modal.selected = (modal.selected + 1).min(entries.len() - 1);
                }
            }
            KeyCode::Char('a') => {
                self.machines_modal = None;
                self.machine_add();
            }
            KeyCode::Enter => {
                let idx = modal.selected;
                self.activate_machines_entry(&entries, idx);
            }
            KeyCode::Char('c') => {
                let idx = modal.selected;
                if let Some(MachineEntry::Configured { name, state, .. }) = entries.get(idx) {
                    self.machines_modal = None;
                    self.connect_or_prompt_login(name, state);
                }
            }
            KeyCode::Char('n') => {
                let idx = modal.selected;
                if let Some(MachineEntry::Configured { name, .. }) = entries.get(idx) {
                    self.machines_modal = None;
                    self.new_workspace_on(Some(name.clone()));
                }
            }
            KeyCode::Char('r') => {
                let idx = modal.selected;
                if let Some(MachineEntry::Configured { name, .. }) = entries.get(idx) {
                    self.machines_modal = None;
                    self.machine_rename(name.clone());
                }
            }
            KeyCode::Char('d') => {
                let idx = modal.selected;
                if let Some(MachineEntry::Configured { name, .. }) = entries.get(idx) {
                    self.machines_modal = None;
                    self.machine_remove(name.clone());
                }
            }
            _ => {}
        }
    }

    pub(super) fn handle_machines_mouse(&mut self, mouse: MouseInput) {
        let pos = Position::new(mouse.col, mouse.row);
        let Some(modal) = self.machines_modal.as_mut() else {
            return;
        };
        match mouse.kind {
            MouseEventKind::Moved | MouseEventKind::Drag(_) => {
                if let Some((_, idx)) = modal.render.rows.iter().find(|(r, _)| r.contains(pos)) {
                    modal.selected = *idx;
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if !modal.render.area.contains(pos) {
                    self.machines_modal = None;
                    return;
                }
                let button = modal
                    .render
                    .buttons
                    .iter()
                    .find(|(r, _)| r.contains(pos))
                    .map(|(_, b)| *b);
                if let Some(button) = button {
                    let selected = modal.selected;
                    let entries = self.machines_modal_entries();
                    self.activate_machines_button(button, &entries, selected);
                    return;
                }
                let hit = modal
                    .render
                    .rows
                    .iter()
                    .find(|(r, _)| r.contains(pos))
                    .map(|(_, idx)| *idx);
                if let Some(idx) = hit {
                    if modal.selected == idx {
                        let entries = self.machines_modal_entries();
                        self.activate_machines_entry(&entries, idx);
                    } else {
                        modal.selected = idx;
                    }
                }
            }
            _ => {}
        }
    }

    fn activate_machines_button(
        &mut self,
        button: MachinesButton,
        entries: &[MachineEntry],
        idx: usize,
    ) {
        match button {
            MachinesButton::Close => self.machines_modal = None,
            MachinesButton::Add => {
                self.machines_modal = None;
                self.machine_add();
            }
            MachinesButton::Connect => self.activate_machines_entry(entries, idx),
            MachinesButton::NewSpace => {
                if let Some(MachineEntry::Configured { name, .. }) = entries.get(idx) {
                    self.machines_modal = None;
                    self.new_workspace_on(Some(name.clone()));
                }
            }
            MachinesButton::Rename => {
                if let Some(MachineEntry::Configured { name, .. }) = entries.get(idx) {
                    self.machines_modal = None;
                    self.machine_rename(name.clone());
                }
            }
            MachinesButton::Delete => {
                if let Some(MachineEntry::Configured { name, .. }) = entries.get(idx) {
                    self.machines_modal = None;
                    self.machine_remove(name.clone());
                }
            }
        }
    }

    fn activate_machines_entry(&mut self, entries: &[MachineEntry], idx: usize) {
        let Some(entry) = entries.get(idx) else {
            return;
        };
        self.machines_modal = None;
        match entry {
            MachineEntry::Configured { name, state, .. } => {
                self.connect_or_prompt_login(name, state);
            }
            MachineEntry::SshHost { alias, .. } => {
                self.submit_action(Action::Machine(MachineOp::Add {
                    target: alias.clone(),
                }));
            }
        }
    }

    /// Reconnect a remote machine, or surface the exact `hc machine login`
    /// command when the core reports `needs_login`.
    pub(super) fn connect_or_prompt_login(&mut self, name: &str, state: &str) {
        if state == "needs_login" {
            self.show_notice(
                "Login required",
                &format!("Run `hc machine login {name}` in a terminal"),
            );
            return;
        }
        self.submit_action(Action::Machine(MachineOp::Connect {
            machine: name.to_owned(),
        }));
    }
}

/// Discover concrete SSH host aliases from `~/.ssh/config` (and `Include`d
/// files up to 8 levels deep), skipping wildcards (`*`, `?`, `!`).
fn discover_ssh_hosts() -> Vec<(String, String)> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let config_path = home.join(".ssh").join("config");
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    parse_ssh_config_file(&config_path, &home, 0, &mut seen, &mut out);
    out
}

#[cfg(test)]
pub(crate) fn parse_ssh_config_text(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    parse_ssh_config_lines(text, None, 0, &mut seen, &mut out);
    out
}

fn parse_ssh_config_file(
    path: &Path,
    home: &Path,
    depth: usize,
    seen: &mut HashSet<String>,
    out: &mut Vec<(String, String)>,
) {
    if depth > 8 {
        return;
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    parse_ssh_config_lines(&text, Some(home), depth, seen, out);
}

fn parse_ssh_config_lines(
    text: &str,
    home: Option<&Path>,
    depth: usize,
    seen: &mut HashSet<String>,
    out: &mut Vec<(String, String)>,
) {
    let mut current: Option<(usize, Option<String>, Option<String>)> = None;
    let flush = |current: &mut Option<(usize, Option<String>, Option<String>)>,
                 out: &mut Vec<(String, String)>| {
        if let Some((idx, user, hostname)) = current.take() {
            let detail = match (user, hostname) {
                (Some(u), Some(h)) => format!("{u}@{h}"),
                (None, Some(h)) => h,
                (Some(u), None) => u,
                (None, None) => String::new(),
            };
            if let Some(slot) = out.get_mut(idx) {
                slot.1 = detail;
            }
        }
    };
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(2, |c: char| c.is_whitespace() || c == '=');
        let Some(key) = parts.next().map(|s| s.trim().to_ascii_lowercase()) else {
            continue;
        };
        let value = parts
            .next()
            .map(|s| s.trim_start_matches('=').trim())
            .unwrap_or_default();
        match key.as_str() {
            "host" | "match" => {
                flush(&mut current, out);
                if key == "host" {
                    let alias = value.split_whitespace().find(|h| {
                        !h.is_empty()
                            && !h.contains(['*', '?', '!'])
                            && seen.insert((*h).to_owned())
                    });
                    if let Some(alias) = alias {
                        let idx = out.len();
                        out.push((alias.to_owned(), String::new()));
                        current = Some((idx, None, None));
                    }
                }
            }
            "hostname" => {
                if let Some((_, _, hostname)) = current.as_mut()
                    && !value.is_empty()
                {
                    *hostname = Some(value.to_owned());
                }
            }
            "user" => {
                if let Some((_, user, _)) = current.as_mut()
                    && !value.is_empty()
                {
                    *user = Some(value.to_owned());
                }
            }
            "include" if depth < 8 => {
                if let Some(home) = home {
                    for token in value.split_whitespace() {
                        let cleaned = token.trim_matches('"');
                        if cleaned.contains(['*', '?', '[']) {
                            continue;
                        }
                        let path = if let Some(rest) = cleaned.strip_prefix("~/") {
                            home.join(rest)
                        } else if Path::new(cleaned).is_absolute() {
                            PathBuf::from(cleaned)
                        } else {
                            home.join(".ssh").join(cleaned)
                        };
                        parse_ssh_config_file(&path, home, depth + 1, seen, out);
                    }
                }
            }
            _ => {}
        }
    }
    flush(&mut current, out);
}
