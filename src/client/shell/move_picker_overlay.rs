use super::*;

pub(super) fn render_move_picker_overlay(
    b: &mut Buffer,
    picker: &ClientMovePickerOverlay,
    p: &Palette,
) -> Option<OverlayRender> {
    // Borders plus title, query, divider, status, gap and buttons around the rows.
    let popup_height = (picker.entries.len() + 8).clamp(10, 24) as u16;
    let popup = popup(b.area, 72, popup_height)?;
    let inner = panel(b, popup, p.accent, p.panel_bg)?;
    put_text(
        b,
        inner.x,
        inner.y,
        inner.width,
        picker.title,
        Style::default()
            .fg(p.text)
            .bg(p.panel_bg)
            .add_modifier(Modifier::BOLD),
    );
    if picker.moves_pane {
        put_right_text(
            b,
            inner,
            inner.y,
            "↵ split right · alt+d split down",
            Style::default().fg(p.overlay0).bg(p.panel_bg),
        );
    }
    let search = Rect::new(inner.x, inner.y + 1, inner.width, 1);
    put_text(
        b,
        search.x,
        search.y,
        search.width,
        " / ",
        Style::default().fg(p.text).bg(p.panel_bg),
    );
    let cursor = text_editor::render(
        b,
        Rect::new(search.x + 3, search.y, search.width.saturating_sub(4), 1),
        &picker.query,
        Style::default().fg(p.text).bg(p.panel_bg),
    );
    put_text(
        b,
        inner.x,
        inner.y + 2,
        inner.width,
        &"─".repeat(inner.width as usize),
        Style::default().fg(p.surface1).bg(p.panel_bg),
    );
    let body = Rect::new(
        inner.x,
        inner.y + 3,
        inner.width,
        inner.height.saturating_sub(6),
    );
    let visible = picker.visible_indices();
    let visible_count = usize::from(body.height).max(1);
    let start = picker
        .selected
        .saturating_sub(visible_count.saturating_sub(1))
        .min(visible.len().saturating_sub(visible_count));
    let name = picker.query.trim();
    let mut row_hits = Vec::new();
    for (row, (position, index)) in visible
        .iter()
        .copied()
        .enumerate()
        .skip(start)
        .take(visible_count)
        .enumerate()
    {
        let (target, label) = &picker.entries[index];
        let rect = Rect::new(body.x, body.y + row as u16, body.width, 1);
        row_hits.push((rect, position));
        let style = if position == picker.selected {
            Style::default().fg(contrast(p)).bg(p.accent)
        } else if target.creates() {
            Style::default().fg(p.overlay0).bg(p.panel_bg)
        } else {
            Style::default().fg(p.text).bg(p.panel_bg)
        };
        b.set_style(rect, style);
        let text = if !target.creates() {
            format!(" {label}")
        } else if name.is_empty() {
            format!(" + {label}")
        } else {
            format!(" + {label} \"{name}\"")
        };
        put_text(b, rect.x, rect.y, rect.width, &text, style);
    }
    if picker.moving {
        put_text(
            b,
            inner.x,
            inner.bottom() - 3,
            inner.width,
            " moving…",
            Style::default().fg(p.accent).bg(p.panel_bg),
        );
    } else if let Some(error) = picker.error.as_deref() {
        put_text(
            b,
            inner.x,
            inner.bottom() - 3,
            inner.width,
            &format!(" {error}"),
            Style::default().fg(p.red).bg(p.panel_bg),
        );
    }
    let buttons = row(inner, &[10, 12], 2, inner.height.saturating_sub(1));
    let [primary, cancel] = buttons.as_slice() else {
        return None;
    };
    button(
        b,
        *primary,
        " ↵ move ",
        Style::default()
            .fg(contrast(p))
            .bg(p.accent)
            .add_modifier(Modifier::BOLD),
    );
    button(
        b,
        *cancel,
        " esc cancel ",
        Style::default()
            .fg(p.text)
            .bg(p.surface0)
            .add_modifier(Modifier::BOLD),
    );
    Some(OverlayRender {
        area: popup,
        primary: *primary,
        cancel: *cancel,
        move_rows: row_hits,
        cursor: cursor.filter(|_| !picker.moving),
        ..OverlayRender::default()
    })
}
