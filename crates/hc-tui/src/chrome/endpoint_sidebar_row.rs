//! One row in the expanded endpoint list: the endpoint label plus its
//! status or auth signal at the right.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use super::palette::Palette;
use super::render::{MachineDiagnostics, display_width, put_right_text, put_text};
use super::snapshot::{ClientEndpointStatus, ClientShellEndpoint};

#[allow(clippy::too_many_arguments)]
pub(super) fn render_endpoint_row(
    buffer: &mut Buffer,
    rect: Rect,
    marker: &str,
    endpoint: &ClientShellEndpoint,
    highlighted: bool,
    mouse_capture: bool,
    auth: &MachineDiagnostics,
    palette: &Palette,
) -> (Rect, Rect) {
    if highlighted {
        buffer.set_style(rect, Style::default().bg(palette.active_row_bg));
    }
    let (glyph, state, color) =
        super::endpoints::endpoint_status_presentation(endpoint.status, palette);
    let state = if endpoint.status == ClientEndpointStatus::Online {
        ""
    } else {
        state
    };
    let signal = if auth.required_for(endpoint) {
        "! auth".to_owned()
    } else if endpoint.status == ClientEndpointStatus::Attention {
        "! error".to_owned()
    } else if endpoint.endpoint_id.is_local() {
        String::new()
    } else if state.is_empty() {
        glyph.to_owned()
    } else {
        format!("{glyph} {state}")
    };
    let signal_width = display_width(&signal).min(rect.width);
    let show_add =
        mouse_capture && endpoint.status == ClientEndpointStatus::Online && rect.width >= 6;
    let add_width = if show_add { 2u16 } else { 0u16 };
    let right_reserve = signal_width.saturating_add(add_width);
    put_text(
        buffer,
        rect.x,
        rect.y,
        rect.width.saturating_sub(right_reserve.saturating_add(1)),
        &format!(" {marker} {}", endpoint.label),
        Style::default()
            .fg(
                if matches!(endpoint.status, ClientEndpointStatus::Disabled) {
                    palette.overlay0
                } else {
                    palette.text
                },
            )
            .add_modifier(Modifier::BOLD),
    );
    let new_workspace = if show_add {
        let add_x = rect
            .right()
            .saturating_sub(signal_width.saturating_add(add_width));
        put_text(
            buffer,
            add_x,
            rect.y,
            add_width,
            "+ ",
            Style::default().fg(palette.overlay0),
        );
        Rect::new(add_x, rect.y, add_width, 1)
    } else {
        Rect::default()
    };
    put_right_text(
        buffer,
        rect,
        rect.y,
        &signal,
        auth.badge_style(endpoint, palette, Style::default().fg(color)),
    );
    let status_badge = Rect::new(
        rect.right().saturating_sub(signal_width),
        rect.y,
        signal_width,
        1,
    );
    (status_badge, new_workspace)
}
