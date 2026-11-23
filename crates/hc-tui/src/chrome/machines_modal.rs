//! TUI-native Machines overlay: configured remote machines with live status
//! and error reasons, plus discovered `~/.ssh/config` hosts ready for one-key
//! `machine.add`.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};

use super::dialogs::{contrast, panel, popup};
use super::palette::Palette;
use super::render::{display_width, put_right_text, put_text};
use super::text::truncate_end;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MachineEntry {
    Configured {
        name: String,
        target: String,
        version: String,
        state: String,
        why: Option<String>,
    },
    SshHost {
        alias: String,
        detail: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MachinesButton {
    Add,
    Connect,
    NewSpace,
    Rename,
    Delete,
    Close,
}

#[derive(Default)]
pub(crate) struct MachinesModalRender {
    pub area: Rect,
    pub rows: Vec<(Rect, usize)>,
    pub buttons: Vec<(Rect, MachinesButton)>,
}

pub(crate) fn render(
    buf: &mut Buffer,
    entries: &[MachineEntry],
    selected: usize,
    palette: &Palette,
) -> Option<MachinesModalRender> {
    let screen = buf.area;
    let configured_count = entries
        .iter()
        .filter(|e| matches!(e, MachineEntry::Configured { .. }))
        .count();
    let ssh_count = entries.len().saturating_sub(configured_count);

    let mut content_rows = 2usize; // title + footer
    if configured_count > 0 {
        content_rows += 1 + configured_count; // header + rows
    } else {
        content_rows += 2; // header + empty hint
    }
    if ssh_count > 0 {
        content_rows += 2 + ssh_count; // blank + header + rows
    }
    // +2 panel borders, +1 blank row below title, +1 blank row above footer.
    let height = (content_rows as u16 + 4).clamp(9, screen.height.saturating_sub(2).max(9));
    let width = 66u16.min(screen.width.saturating_sub(4).max(24));
    let area = popup(screen, width, height)?;
    let inner = panel(buf, area, palette.accent, palette.panel_bg)?;
    if inner.height < 4 || inner.width < 10 {
        return None;
    }

    put_text(
        buf,
        inner.x + 1,
        inner.y,
        inner.width.saturating_sub(2),
        "machines",
        Style::default()
            .fg(palette.text)
            .bg(palette.panel_bg)
            .add_modifier(Modifier::BOLD),
    );

    let footer_y = inner.bottom().saturating_sub(1);
    let body_bottom = footer_y.saturating_sub(1);
    let mut y = inner.y + 2;
    let mut rows = Vec::new();

    // Section 1: Configured machines
    if y < body_bottom {
        put_text(
            buf,
            inner.x + 1,
            y,
            inner.width.saturating_sub(2),
            "configured",
            Style::default()
                .fg(palette.overlay0)
                .bg(palette.panel_bg)
                .add_modifier(Modifier::BOLD),
        );
        y += 1;
    }
    if configured_count == 0 {
        if y < body_bottom {
            put_text(
                buf,
                inner.x + 2,
                y,
                inner.width.saturating_sub(3),
                "no remote machines yet — press a to add or pick below",
                Style::default()
                    .fg(palette.overlay0)
                    .bg(palette.panel_bg)
                    .add_modifier(Modifier::DIM),
            );
            y += 1;
        }
    } else {
        for (idx, entry) in entries.iter().enumerate() {
            let MachineEntry::Configured {
                name,
                target,
                version,
                state,
                why,
            } = entry
            else {
                continue;
            };
            if y >= body_bottom {
                break;
            }
            let row_rect = Rect::new(inner.x, y, inner.width, 1);
            let is_sel = idx == selected;
            let (glyph, state_color) = machine_state_glyph(state, palette);
            let bg = if is_sel {
                palette.accent
            } else {
                palette.panel_bg
            };
            let fg = if is_sel {
                contrast(palette)
            } else {
                palette.text
            };
            let dim_fg = if is_sel {
                contrast(palette)
            } else {
                palette.overlay0
            };
            let base = Style::default().fg(fg).bg(bg);
            buf.set_style(row_rect, base);
            put_text(
                buf,
                row_rect.x + 1,
                y,
                2,
                glyph,
                Style::default()
                    .fg(if is_sel { fg } else { state_color })
                    .bg(bg),
            );
            let status_text = if state == "connected" {
                if version.is_empty() {
                    "connected".to_owned()
                } else {
                    format!("v{version}")
                }
            } else if let Some(reason) = why.as_deref().filter(|s| !s.is_empty()) {
                format!("{} · {}", state.replace('_', " "), reason)
            } else {
                state.replace('_', " ")
            };
            let right_w = display_width(&status_text).min(row_rect.width / 2);
            let left_budget = row_rect.width.saturating_sub(right_w + 5) as usize;
            let left_text = if target.is_empty() || target == name {
                name.clone()
            } else {
                format!("{name}  {target}")
            };
            put_text(
                buf,
                row_rect.x + 3,
                y,
                left_budget as u16,
                &truncate_end(&left_text, left_budget),
                if is_sel {
                    base.add_modifier(Modifier::BOLD)
                } else {
                    base
                },
            );
            put_right_text(
                buf,
                Rect::new(row_rect.x, y, row_rect.width.saturating_sub(1), 1),
                y,
                &truncate_end(&status_text, right_w as usize),
                Style::default()
                    .fg(if is_sel {
                        fg
                    } else if state == "connected" {
                        dim_fg
                    } else {
                        state_color
                    })
                    .bg(bg),
            );
            rows.push((row_rect, idx));
            y += 1;
        }
    }

    // Section 2: Discovered ~/.ssh/config hosts
    if ssh_count > 0 && y + 1 < body_bottom {
        y += 1;
        put_text(
            buf,
            inner.x + 1,
            y,
            inner.width.saturating_sub(2),
            "ssh config (enter to add)",
            Style::default()
                .fg(palette.overlay0)
                .bg(palette.panel_bg)
                .add_modifier(Modifier::BOLD),
        );
        y += 1;
        for (idx, entry) in entries.iter().enumerate() {
            let MachineEntry::SshHost { alias, detail } = entry else {
                continue;
            };
            if y >= body_bottom {
                break;
            }
            let row_rect = Rect::new(inner.x, y, inner.width, 1);
            let is_sel = idx == selected;
            let bg = if is_sel {
                palette.accent
            } else {
                palette.panel_bg
            };
            let fg = if is_sel {
                contrast(palette)
            } else {
                palette.text
            };
            let base = Style::default().fg(fg).bg(bg);
            buf.set_style(row_rect, base);
            let label = if detail.is_empty() {
                format!("+ {alias}")
            } else {
                format!("+ {alias}  {detail}")
            };
            put_text(
                buf,
                row_rect.x + 1,
                y,
                row_rect.width.saturating_sub(2),
                &truncate_end(&label, row_rect.width.saturating_sub(2) as usize),
                if is_sel {
                    base.add_modifier(Modifier::BOLD)
                } else {
                    base
                },
            );
            rows.push((row_rect, idx));
            y += 1;
        }
    }

    // Clickable action buttons in the footer bar
    let specs: &[(&str, MachinesButton)] = match entries.get(selected) {
        Some(MachineEntry::Configured { .. }) => &[
            ("c connect", MachinesButton::Connect),
            ("n +space", MachinesButton::NewSpace),
            ("r rename", MachinesButton::Rename),
            ("d delete", MachinesButton::Delete),
            ("a +add", MachinesButton::Add),
            ("esc", MachinesButton::Close),
        ],
        Some(MachineEntry::SshHost { .. }) => &[
            ("↵ add host", MachinesButton::Connect),
            ("a +add", MachinesButton::Add),
            ("esc close", MachinesButton::Close),
        ],
        None => &[
            ("a +add machine", MachinesButton::Add),
            ("esc close", MachinesButton::Close),
        ],
    };
    let mut buttons = Vec::new();
    let mut bx = inner.x + 1;
    let right_limit = inner.right().saturating_sub(1);
    for &(label, kind) in specs {
        let w = display_width(label);
        if bx + w > right_limit {
            break;
        }
        let rect = Rect::new(bx, footer_y, w, 1);
        let style = match kind {
            MachinesButton::Delete => Style::default()
                .fg(palette.red)
                .bg(palette.panel_bg)
                .add_modifier(Modifier::BOLD),
            MachinesButton::Add | MachinesButton::Connect => Style::default()
                .fg(palette.accent)
                .bg(palette.panel_bg)
                .add_modifier(Modifier::BOLD),
            _ => Style::default().fg(palette.text).bg(palette.panel_bg),
        };
        put_text(buf, rect.x, rect.y, rect.width, label, style);
        buttons.push((rect, kind));
        bx = bx.saturating_add(w + 2);
    }

    Some(MachinesModalRender {
        area,
        rows,
        buttons,
    })
}

fn machine_state_glyph(state: &str, palette: &Palette) -> (&'static str, ratatui::style::Color) {
    match state {
        "connected" => ("●", palette.green),
        "connecting" => ("◐", palette.yellow),
        "needs_login" | "unreachable" => ("!", palette.red),
        _ => ("·", palette.overlay0),
    }
}
