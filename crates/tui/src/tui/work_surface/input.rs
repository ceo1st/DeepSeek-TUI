use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use crate::tui::app::{App, SidebarRowAction};

use super::interaction::{
    activate_primary, activate_stop, claim_focus, close_opened, disarm_stop, on_selection_changed,
    release_focus,
};
use super::model::{WorkRow, WorkRowId, project};

#[derive(Debug, Default)]
pub struct MouseOutcome {
    pub consumed: bool,
    pub action: Option<SidebarRowAction>,
}

/// Handle the work surface's focused keyboard contract. `Alt+W` enters the
/// surface from the composer; Esc returns ownership to the composer (or clears
/// a local stop arm / open detail first). Plain printable input always returns
/// ownership to the composer instead of becoming a hidden panel shortcut.
pub fn handle_key(app: &mut App, key: KeyEvent) -> Option<Option<SidebarRowAction>> {
    let rows = project(app);
    if rows.is_empty() {
        return None;
    }
    if !app.work_surface.focused {
        if key.code == KeyCode::Char('w') && key.modifiers.contains(KeyModifiers::ALT) {
            claim_focus(app);
            app.work_surface.clamp_selection(&rows);
            app.needs_redraw = true;
            return Some(None);
        }
        return None;
    }

    if matches!(key.code, KeyCode::Char(_))
        && !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
    {
        release_focus(app);
        return None;
    }

    let action = match key.code {
        KeyCode::Esc => {
            if app.work_surface.stop_arm.is_some() {
                disarm_stop(app);
            } else if app.work_surface.opened.is_some() {
                close_opened(app);
            } else {
                release_focus(app);
            }
            return Some(None);
        }
        KeyCode::Up => {
            move_selection(app, &rows, -1);
            on_selection_changed(app);
            None
        }
        KeyCode::Down => {
            move_selection(app, &rows, 1);
            on_selection_changed(app);
            None
        }
        KeyCode::Home => {
            select_edge(app, &rows, false);
            on_selection_changed(app);
            None
        }
        KeyCode::End => {
            select_edge(app, &rows, true);
            on_selection_changed(app);
            None
        }
        KeyCode::PageUp => {
            move_selection(app, &rows, -(app.work_surface.visible_rows.max(1) as isize));
            on_selection_changed(app);
            None
        }
        KeyCode::PageDown => {
            move_selection(app, &rows, app.work_surface.visible_rows.max(1) as isize);
            on_selection_changed(app);
            None
        }
        KeyCode::Delete => selected_row(app, &rows).and_then(|row| {
            row.stop_action
                .clone()
                .and_then(|action| activate_stop(app, &row.id, action))
        }),
        KeyCode::Enter => {
            // Enter confirms an armed Stop on the selected row; otherwise it
            // toggles the primary Open/detail action.
            if let Some(arm) = app.work_surface.stop_arm.as_ref()
                && arm.is_active()
                && app.work_surface.selected.as_ref() == Some(&arm.row_id)
            {
                let row_id = arm.row_id.clone();
                let action = arm.action.clone();
                activate_stop(app, &row_id, action)
            } else {
                selected_row(app, &rows)
                    .and_then(|row| activate_primary(app, &row.id, row.primary_action.clone()))
            }
        }
        _ => return None,
    };
    app.work_surface.clamp_selection(&rows);
    app.needs_redraw = true;
    Some(action)
}

pub fn handle_mouse(app: &mut App, mouse: MouseEvent) -> MouseOutcome {
    let Some(area) = app.work_surface.last_area else {
        return MouseOutcome::default();
    };
    let inside = mouse.column >= area.x
        && mouse.column < area.right()
        && mouse.row >= area.y
        && mouse.row < area.bottom();
    if !inside {
        if matches!(
            mouse.kind,
            MouseEventKind::Down(MouseButton::Left)
                | MouseEventKind::ScrollUp
                | MouseEventKind::ScrollDown
        ) && app.work_surface.focused
        {
            // Another region is taking the pointer — release strip focus so
            // only one owner shows selection.
            release_focus(app);
        }
        if matches!(mouse.kind, MouseEventKind::Moved) && app.work_surface.hovered.take().is_some()
        {
            app.needs_redraw = true;
        }
        return MouseOutcome::default();
    }

    match mouse.kind {
        MouseEventKind::ScrollUp => {
            claim_focus(app);
            app.work_surface.scroll_offset = app.work_surface.scroll_offset.saturating_sub(2);
            app.needs_redraw = true;
            MouseOutcome {
                consumed: true,
                action: None,
            }
        }
        MouseEventKind::ScrollDown => {
            claim_focus(app);
            let max = app
                .work_surface
                .total_rows
                .saturating_sub(app.work_surface.visible_rows.max(1));
            app.work_surface.scroll_offset =
                app.work_surface.scroll_offset.saturating_add(2).min(max);
            app.needs_redraw = true;
            MouseOutcome {
                consumed: true,
                action: None,
            }
        }
        MouseEventKind::Moved => {
            let hovered = hit_row(app, mouse.row).map(|row| row.id.clone());
            if app.work_surface.hovered != hovered {
                app.work_surface.hovered = hovered;
                app.needs_redraw = true;
            }
            MouseOutcome {
                consumed: true,
                action: None,
            }
        }
        MouseEventKind::Down(MouseButton::Left) => {
            let row = hit_row(app, mouse.row).cloned();
            let Some(row) = row else {
                claim_focus(app);
                return MouseOutcome {
                    consumed: true,
                    action: None,
                };
            };
            claim_focus(app);
            let hitbox = app
                .work_surface
                .hitboxes
                .iter()
                .find(|candidate| candidate.row_y == mouse.row)
                .cloned();

            let in_stop = hitbox.as_ref().is_some_and(|hit| {
                hit.stop_zone_start_col
                    .zip(hit.stop_zone_end_col)
                    .is_some_and(|(start, end)| mouse.column >= start && mouse.column < end)
            });
            let in_open = hitbox.as_ref().is_some_and(|hit| {
                hit.open_zone_start_col
                    .zip(hit.open_zone_end_col)
                    .is_some_and(|(start, end)| mouse.column >= start && mouse.column < end)
            });

            let previous = app.work_surface.selected.clone();
            app.work_surface.selected = Some(row.id.clone());
            if previous.as_ref() != Some(&row.id) {
                on_selection_changed(app);
            }
            app.needs_redraw = true;

            let action = if in_stop {
                row.stop_action
                    .clone()
                    .and_then(|action| activate_stop(app, &row.id, action))
            } else if in_open || row.primary_action.is_some() {
                // Open zone and row body share the primary activate/toggle.
                activate_primary(app, &row.id, row.primary_action.clone())
            } else {
                None
            };
            MouseOutcome {
                consumed: true,
                action,
            }
        }
        _ => MouseOutcome {
            consumed: true,
            action: None,
        },
    }
}

fn hit_row(app: &App, row_y: u16) -> Option<&WorkRow> {
    let id = app
        .work_surface
        .hitboxes
        .iter()
        .find(|hitbox| hitbox.row_y == row_y)
        .map(|hitbox| &hitbox.id)?;
    app.work_surface
        .latest_rows
        .iter()
        .find(|row| &row.id == id)
}

fn selected_row<'a>(app: &App, rows: &'a [WorkRow]) -> Option<&'a WorkRow> {
    let selected = app.work_surface.selected.as_ref()?;
    rows.iter().find(|row| &row.id == selected)
}

fn selectable_ids(rows: &[WorkRow]) -> Vec<WorkRowId> {
    rows.iter()
        .filter(|row| row.selectable)
        .map(|row| row.id.clone())
        .collect()
}

fn move_selection(app: &mut App, rows: &[WorkRow], delta: isize) {
    let ids = selectable_ids(rows);
    if ids.is_empty() {
        return;
    }
    let current = app
        .work_surface
        .selected
        .as_ref()
        .and_then(|selected| ids.iter().position(|id| id == selected))
        .unwrap_or_default();
    let next = if delta.is_negative() {
        current.saturating_sub(delta.unsigned_abs())
    } else {
        current
            .saturating_add(delta as usize)
            .min(ids.len().saturating_sub(1))
    };
    app.work_surface.selected = Some(ids[next].clone());
}

fn select_edge(app: &mut App, rows: &[WorkRow], end: bool) {
    let ids = selectable_ids(rows);
    app.work_surface.selected = if end {
        ids.last().cloned()
    } else {
        ids.first().cloned()
    };
}
