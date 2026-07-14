//! Projection of an accepted Plan into the independent Work/To-do store.

use crate::tools::plan::{PlanSnapshot, SharedPlanState, StepStatus};
use crate::tools::todo::{SharedTodoList, TodoList, TodoListSnapshot, TodoStatus};

const PROVENANCE_PREFIX: &str = "\u{2063}cw-plan-step:";
const PROVENANCE_SUFFIX: &str = "\u{2063}";

/// The outcomes that can follow the Plan confirmation prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanAcceptance {
    AcceptAct,
    AcceptFullAccess,
    Revise,
    Exit,
}

impl PlanAcceptance {
    fn is_accept(self) -> bool {
        matches!(self, Self::AcceptAct | Self::AcceptFullAccess)
    }
}

/// Project an accepted plan into the existing To-do list.
///
/// Plan and To-do remain separate stores. The plan is read once, then only
/// rows carrying this bridge's provenance marker are created or updated.
pub async fn project_accepted_plan(
    plan_state: &SharedPlanState,
    todo_list: &SharedTodoList,
    acceptance: PlanAcceptance,
) -> Result<usize, String> {
    if !acceptance.is_accept() {
        return Ok(0);
    }

    let plan = plan_state.lock().await.snapshot();
    let mut todos = todo_list.lock().await;
    let mut snapshot = todos.snapshot();
    let changed = project_plan_snapshot(&plan, &mut snapshot)?;
    *todos = TodoList::from_snapshot(&snapshot)?;
    Ok(changed)
}

/// Project a plan snapshot into a To-do snapshot.
///
/// The ordinal is the stable identity available from the current Plan
/// contract: Plan steps do not carry IDs. It is deliberately stored as an
/// invisible marker so ordinary To-do rows remain unchanged to users.
pub fn project_plan_snapshot(
    plan: &PlanSnapshot,
    todos: &mut TodoListSnapshot,
) -> Result<usize, String> {
    let mut list = TodoList::from_snapshot(todos)?;
    let mut changed = 0;

    for (index, step) in plan.items.iter().enumerate() {
        let marker = provenance_marker(index);
        let content = format!("{}{}", step.step.trim(), marker);
        let status = todo_status(step.status.clone());

        if let Some(item) = list
            .snapshot()
            .items
            .iter()
            .find(|item| provenance_index(&item.content) == Some(index))
            .cloned()
        {
            let mut updated = false;
            let content_changed = item.content != content;
            if content_changed {
                replace_content(&mut list, item.id, content);
                updated = true;
            }
            // Preserve a user's completed projection only while it still
            // represents the same plan step. Ordinals are the only stable IDs
            // in the current Plan contract, so an inserted/replaced step at the
            // same ordinal must take the new plan status instead of inheriting
            // a stale completion.
            if item.status != status && (item.status != TodoStatus::Completed || content_changed) {
                list.update_status(item.id, status);
                updated = true;
            }
            if updated {
                changed += 1;
            }
        } else {
            list.add(content, status);
            changed += 1;
        }
    }

    let mut projected = list.snapshot();
    let before_retain = projected.items.len();
    projected.items.retain(|item| {
        provenance_index(&item.content).is_none_or(|index| index < plan.items.len())
    });
    changed += before_retain.saturating_sub(projected.items.len());
    // Rebuild once more so derived fields and the single-in-progress invariant
    // are computed from the pruned projection.
    *todos = TodoList::from_snapshot(&projected)?.snapshot();
    Ok(changed)
}

fn todo_status(status: StepStatus) -> TodoStatus {
    match status {
        StepStatus::Pending => TodoStatus::Pending,
        StepStatus::InProgress => TodoStatus::InProgress,
        StepStatus::Completed => TodoStatus::Completed,
    }
}

fn provenance_marker(index: usize) -> String {
    format!("{PROVENANCE_PREFIX}{index}{PROVENANCE_SUFFIX}")
}

fn provenance_index(content: &str) -> Option<usize> {
    let start = content.find(PROVENANCE_PREFIX)? + PROVENANCE_PREFIX.len();
    let end = content[start..].find(PROVENANCE_SUFFIX)? + start;
    content[start..end].parse().ok()
}

fn replace_content(list: &mut TodoList, id: u32, content: String) {
    let mut snapshot = list.snapshot();
    if let Some(item) = snapshot.items.iter_mut().find(|item| item.id == id) {
        item.content = content;
    }
    if let Ok(rebuilt) = TodoList::from_snapshot(&snapshot) {
        *list = rebuilt;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::plan::{PlanItemArg, StepStatus};
    use crate::tools::todo::{TodoItem, TodoStatus};

    fn plan(items: &[(&str, StepStatus)]) -> PlanSnapshot {
        PlanSnapshot {
            items: items
                .iter()
                .map(|(step, status)| PlanItemArg {
                    step: (*step).to_string(),
                    status: status.clone(),
                })
                .collect(),
            ..PlanSnapshot::default()
        }
    }

    fn todos(items: &[(&str, TodoStatus)]) -> TodoListSnapshot {
        TodoListSnapshot {
            items: items
                .iter()
                .enumerate()
                .map(|(id, (content, status))| TodoItem {
                    id: u32::try_from(id + 1).expect("test id"),
                    content: (*content).to_string(),
                    status: *status,
                })
                .collect(),
            ..TodoListSnapshot::default()
        }
    }

    #[test]
    fn projection_creates_first_class_rows() {
        let mut actual = TodoListSnapshot::default();
        let changed = project_plan_snapshot(
            &plan(&[
                ("Inspect", StepStatus::Pending),
                ("Ship", StepStatus::Completed),
            ]),
            &mut actual,
        )
        .expect("projection");
        assert_eq!(changed, 2);
        assert_eq!(actual.items.len(), 2);
        assert_eq!(actual.items[0].status, TodoStatus::Pending);
        assert_eq!(actual.items[1].status, TodoStatus::Completed);
        assert_eq!(provenance_index(&actual.items[0].content), Some(0));
    }

    #[test]
    fn repeated_acceptance_is_idempotent_and_updates_changed_steps() {
        let mut actual = TodoListSnapshot::default();
        let first = plan(&[("Old", StepStatus::Pending)]);
        project_plan_snapshot(&first, &mut actual).expect("first projection");
        assert_eq!(
            project_plan_snapshot(&first, &mut actual).expect("repeat projection"),
            0
        );
        let second = plan(&[("New", StepStatus::InProgress)]);
        let changed = project_plan_snapshot(&second, &mut actual).expect("reprojection");
        assert_eq!(changed, 1);
        assert_eq!(actual.items.len(), 1);
        assert!(actual.items[0].content.starts_with("New"));
        assert_eq!(actual.items[0].status, TodoStatus::InProgress);
        assert_eq!(provenance_index(&actual.items[0].content), Some(0));
    }

    #[test]
    fn projection_preserves_user_rows_and_order() {
        let mut actual = todos(&[("User row", TodoStatus::Pending)]);
        project_plan_snapshot(&plan(&[("Plan row", StepStatus::Pending)]), &mut actual)
            .expect("projection");
        assert_eq!(actual.items[0].content, "User row");
        assert!(actual.items[1].content.starts_with("Plan row"));
    }

    #[test]
    fn completed_projected_rows_are_not_resurrected() {
        let mut actual = TodoListSnapshot::default();
        project_plan_snapshot(&plan(&[("Done", StepStatus::Completed)]), &mut actual)
            .expect("first projection");
        project_plan_snapshot(&plan(&[("Done", StepStatus::Pending)]), &mut actual)
            .expect("reprojection");
        assert_eq!(actual.items[0].status, TodoStatus::Completed);
    }

    #[test]
    fn replaced_step_does_not_inherit_completion_from_the_same_ordinal() {
        let mut actual = TodoListSnapshot::default();
        project_plan_snapshot(&plan(&[("Old", StepStatus::Completed)]), &mut actual)
            .expect("first projection");
        project_plan_snapshot(&plan(&[("New", StepStatus::Pending)]), &mut actual)
            .expect("replacement projection");
        assert!(actual.items[0].content.starts_with("New"));
        assert_eq!(actual.items[0].status, TodoStatus::Pending);
    }

    #[test]
    fn shorter_plan_retires_only_stale_projected_rows() {
        let mut actual = todos(&[("User row", TodoStatus::Pending)]);
        project_plan_snapshot(
            &plan(&[
                ("Keep", StepStatus::Pending),
                ("Remove", StepStatus::Pending),
            ]),
            &mut actual,
        )
        .expect("first projection");
        let changed = project_plan_snapshot(&plan(&[("Keep", StepStatus::Pending)]), &mut actual)
            .expect("shorter projection");
        assert_eq!(changed, 1);
        assert_eq!(actual.items.len(), 2);
        assert_eq!(actual.items[0].content, "User row");
        assert!(actual.items[1].content.starts_with("Keep"));
    }

    #[test]
    fn revise_and_exit_are_no_ops() {
        let actual = todos(&[("User row", TodoStatus::Completed)]);
        let before = actual.clone();
        assert!(!PlanAcceptance::Revise.is_accept());
        assert!(!PlanAcceptance::Exit.is_accept());
        assert_eq!(actual, before);
    }

    #[tokio::test]
    async fn revise_and_exit_do_not_mutate_shared_todos() {
        let plan_state = crate::tools::plan::new_shared_plan_state();
        plan_state
            .lock()
            .await
            .update(crate::tools::plan::UpdatePlanArgs {
                plan: vec![PlanItemArg {
                    step: "Do not project".to_string(),
                    status: StepStatus::Pending,
                }],
                ..crate::tools::plan::UpdatePlanArgs::default()
            });
        let todo_list = crate::tools::todo::new_shared_todo_list();
        todo_list
            .lock()
            .await
            .add("User row".to_string(), TodoStatus::Pending);
        let before = todo_list.lock().await.snapshot();

        project_accepted_plan(&plan_state, &todo_list, PlanAcceptance::Revise)
            .await
            .expect("revise no-op");
        project_accepted_plan(&plan_state, &todo_list, PlanAcceptance::Exit)
            .await
            .expect("exit no-op");

        assert_eq!(todo_list.lock().await.snapshot(), before);
    }
}
