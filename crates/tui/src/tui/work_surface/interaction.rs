//! Typed work-surface interaction ownership (TUI-DOG-004 / 005 / 006).
//!
//! Selection, focus, detail-open, and destructive stop-arm are distinct axes.
//! Hitboxes are recorded during render; this module only owns state transitions.

use std::time::{Duration, Instant};

use crate::tui::app::{App, SidebarRowAction};
use crate::tui::views::ModalKind;

use super::model::WorkRowId;

/// How long a row-local Stop stays armed before it expires.
pub const STOP_ARM_WINDOW: Duration = Duration::from_secs(4);

/// Row-local destructive arm. Confirmation lives on the worker row — never in
/// a distant footer-only prompt.
#[derive(Debug, Clone)]
pub struct StopArm {
    pub row_id: WorkRowId,
    pub until: Instant,
    pub action: SidebarRowAction,
}

impl StopArm {
    #[must_use]
    pub fn is_active(&self) -> bool {
        Instant::now() < self.until
    }
}

/// Claim work-surface focus and clear competing selection owners.
pub fn claim_focus(app: &mut App) {
    let was_focused = app.work_surface.focused;
    app.work_surface.focused = true;
    if app.viewport.transcript_selection.is_active() {
        app.viewport.transcript_selection.clear();
    }
    if !was_focused {
        app.needs_redraw = true;
    }
}

/// Release work-surface focus without clearing the remembered selection.
pub fn release_focus(app: &mut App) {
    if !app.work_surface.focused && app.work_surface.hovered.is_none() {
        return;
    }
    app.work_surface.focused = false;
    app.work_surface.hovered = None;
    disarm_stop(app);
    app.needs_redraw = true;
}

pub fn disarm_stop(app: &mut App) {
    if app.work_surface.stop_arm.take().is_some() {
        app.needs_redraw = true;
    }
}

/// Expire an armed Stop after its window. Returns true when state changed.
pub fn tick_stop_arm(app: &mut App) -> bool {
    let expired = app
        .work_surface
        .stop_arm
        .as_ref()
        .is_some_and(|arm| !arm.is_active());
    if expired {
        app.work_surface.stop_arm = None;
        app.needs_redraw = true;
        true
    } else {
        false
    }
}

/// Arm Stop for `row_id`, replacing any prior arm.
pub fn arm_stop(app: &mut App, row_id: WorkRowId, action: SidebarRowAction) {
    app.work_surface.stop_arm = Some(StopArm {
        row_id,
        until: Instant::now() + STOP_ARM_WINDOW,
        action,
    });
    app.needs_redraw = true;
}

/// First activation arms; second activation on the same armed row confirms.
pub fn activate_stop(
    app: &mut App,
    row_id: &WorkRowId,
    action: SidebarRowAction,
) -> Option<SidebarRowAction> {
    let already_armed = app
        .work_surface
        .stop_arm
        .as_ref()
        .is_some_and(|arm| arm.is_active() && &arm.row_id == row_id);
    if already_armed {
        let confirm = app
            .work_surface
            .stop_arm
            .take()
            .map(|arm| arm.action)
            .unwrap_or(action);
        app.work_surface.stopping = Some(row_id.clone());
        app.work_surface.opened = None;
        app.needs_redraw = true;
        Some(confirm)
    } else {
        arm_stop(app, row_id.clone(), action);
        None
    }
}

/// Open or toggle-close the primary detail for a row.
///
/// Enter/click on an already-opened selected row closes it. Opening a different
/// row clears any stop arm so destructive confirm stays row-local.
pub fn activate_primary(
    app: &mut App,
    row_id: &WorkRowId,
    primary: Option<SidebarRowAction>,
) -> Option<SidebarRowAction> {
    disarm_stop(app);
    if app.work_surface.opened.as_ref() == Some(row_id) {
        close_opened(app);
        return None;
    }
    app.work_surface.selected = Some(row_id.clone());
    let action = primary?;
    app.work_surface.opened = Some(row_id.clone());
    Some(action)
}

/// Close the work-surface-owned detail (pager when we opened it).
pub fn close_opened(app: &mut App) {
    if app.work_surface.opened.take().is_none() {
        return;
    }
    if app.view_stack.top_kind() == Some(ModalKind::Pager) {
        app.view_stack.pop();
    }
    app.needs_redraw = true;
}

/// Moving selection/focus away from an armed row clears the arm.
pub fn on_selection_changed(app: &mut App) {
    let Some(armed_row) = app
        .work_surface
        .stop_arm
        .as_ref()
        .filter(|arm| arm.is_active())
        .map(|arm| arm.row_id.clone())
    else {
        return;
    };
    if app.work_surface.selected.as_ref() != Some(&armed_row) {
        disarm_stop(app);
    }
}

/// Drop the transient "stopping…" marker once the worker leaves the projection
/// or is no longer stoppable.
pub fn clear_stale_stopping(app: &mut App, stoppable_ids: &[WorkRowId]) {
    let Some(stopping) = app.work_surface.stopping.as_ref() else {
        return;
    };
    if !stoppable_ids.iter().any(|id| id == stopping) {
        app.work_surface.stopping = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::tui::app::TuiOptions;
    use std::path::PathBuf;

    fn app() -> App {
        let options = TuiOptions {
            model: "deepseek-v4-pro".to_string(),
            workspace: PathBuf::from("."),
            config_path: None,
            config_profile: None,
            allow_shell: false,
            use_alt_screen: true,
            use_mouse_capture: true,
            use_bracketed_paste: true,
            max_subagents: 4,
            skills_dir: PathBuf::from("."),
            memory_path: PathBuf::from("memory.md"),
            notes_path: PathBuf::from("notes.txt"),
            mcp_config_path: PathBuf::from("mcp.json"),
            use_memory: false,
            start_in_agent_mode: false,
            skip_onboarding: true,
            yolo: false,
            resume_session_id: None,
            initial_input: None,
        };
        App::new(options, &Config::default())
    }

    #[test]
    fn stop_arms_then_confirms_on_second_activation() {
        let mut app = app();
        let row = WorkRowId("worker:a1".into());
        let action = SidebarRowAction::CancelAgent {
            agent_id: "a1".into(),
        };
        assert!(activate_stop(&mut app, &row, action.clone()).is_none());
        assert!(app.work_surface.stop_arm.is_some());
        let confirmed = activate_stop(&mut app, &row, action).expect("confirm");
        assert!(matches!(
            confirmed,
            SidebarRowAction::CancelAgent { agent_id } if agent_id == "a1"
        ));
        assert!(app.work_surface.stop_arm.is_none());
        assert_eq!(app.work_surface.stopping.as_ref(), Some(&row));
    }

    #[test]
    fn primary_toggles_opened_closed() {
        let mut app = app();
        let row = WorkRowId("worker:a1".into());
        let open = SidebarRowAction::OpenAgentDetail {
            agent_id: "a1".into(),
        };
        assert!(activate_primary(&mut app, &row, Some(open.clone())).is_some());
        assert_eq!(app.work_surface.opened.as_ref(), Some(&row));
        assert!(activate_primary(&mut app, &row, Some(open)).is_none());
        assert!(app.work_surface.opened.is_none());
    }

    #[test]
    fn claim_focus_clears_transcript_selection() {
        use crate::tui::selection::TranscriptSelectionPoint;
        let mut app = app();
        app.viewport.transcript_selection.anchor = Some(TranscriptSelectionPoint {
            line_index: 0,
            column: 0,
        });
        app.viewport.transcript_selection.head = app.viewport.transcript_selection.anchor;
        claim_focus(&mut app);
        assert!(app.work_surface.focused);
        assert!(!app.viewport.transcript_selection.is_active());
    }
}
