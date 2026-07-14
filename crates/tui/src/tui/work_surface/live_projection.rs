//! Canonical live-work projection for the Ocean work surface.
//!
//! This is deliberately a read-only projection.  The task panel, active tool
//! cell, worker cache, and workflow panel remain the owners of their state;
//! this module owns only the identity, kind, liveness, and ordering used by
//! the work surface.

use std::collections::HashMap;

use crate::tools::subagent::{AgentWorkerStatus, SubAgentStatus};
use crate::tui::app::{App, TaskPanelEntry, TaskPanelEntryKind};
use crate::tui::history::{HistoryCell, ToolCell, ToolStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LiveWorkKind {
    Task,
    Run,
    Worker,
    Workflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum LiveWorkState {
    Active,
    Waiting,
    Settled,
}

impl LiveWorkState {
    fn rank(self) -> u8 {
        match self {
            Self::Active => 0,
            Self::Waiting => 1,
            Self::Settled => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LiveWorkRow {
    pub identity: String,
    pub kind: LiveWorkKind,
    pub state: LiveWorkState,
    pub status: String,
    pub label: String,
    pub detail: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct LiveWorkCounts {
    pub active: usize,
    pub tasks: usize,
    pub runs: usize,
    pub workers: usize,
    pub workflows: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct LiveWorkProjection {
    pub rows: Vec<LiveWorkRow>,
    pub counts: LiveWorkCounts,
}

impl LiveWorkProjection {
    #[must_use]
    pub(super) fn from_app(app: &App) -> Self {
        let mut by_identity = HashMap::new();

        for task in &app.task_panel {
            let (kind, identity) = if task.kind == TaskPanelEntryKind::Background {
                if let Some(shell_id) = shell_id_from_task(task) {
                    (LiveWorkKind::Run, format!("shell:{shell_id}"))
                } else {
                    (LiveWorkKind::Task, format!("task:{}", task.id))
                }
            } else {
                (LiveWorkKind::Task, format!("task:{}", task.id))
            };
            insert_prefer_live(&mut by_identity, row_from_task(task, kind, identity));
        }

        // Wait/tool cards are a second representation of the same shell job.
        // Their task_id is the stable identity; never add a second visible row.
        if let Some(active) = app.active_cell.as_ref() {
            for cell in active.entries() {
                match cell {
                    HistoryCell::Tool(ToolCell::Exec(exec))
                        if exec.status == ToolStatus::Running =>
                    {
                        if let Some(shell_id) = exec.shell_task_id.as_deref() {
                            insert_prefer_live(
                                &mut by_identity,
                                LiveWorkRow {
                                    identity: format!("shell:{shell_id}"),
                                    kind: LiveWorkKind::Run,
                                    state: LiveWorkState::Active,
                                    status: "running".to_string(),
                                    label: format!("shell: {}", exec.command),
                                    detail: shell_id.to_string(),
                                },
                            );
                        }
                    }
                    HistoryCell::Tool(ToolCell::Generic(tool))
                        if tool.status == ToolStatus::Running && is_shell_wait_tool(&tool.name) =>
                    {
                        if let Some(shell_id) =
                            shell_id_from_text(tool.input_summary.as_deref().unwrap_or_default())
                        {
                            insert_prefer_live(
                                &mut by_identity,
                                LiveWorkRow {
                                    identity: format!("shell:{shell_id}"),
                                    kind: LiveWorkKind::Run,
                                    state: LiveWorkState::Waiting,
                                    status: "waiting".to_string(),
                                    label: "shell wait".to_string(),
                                    detail: shell_id,
                                },
                            );
                        }
                    }
                    _ => {}
                }
            }
        }

        for agent in &app.subagent_cache {
            let status = agent
                .worker_status
                .map(worker_status)
                .unwrap_or_else(|| subagent_status(&agent.status));
            let state = worker_state(agent.worker_status, &agent.status, status);
            let name = agent
                .nickname
                .clone()
                .or_else(|| app.agent_label_map.get(&agent.agent_id).cloned())
                .unwrap_or_else(|| agent.name.clone());
            insert_prefer_live(
                &mut by_identity,
                LiveWorkRow {
                    identity: format!("worker:{}", agent.agent_id),
                    kind: LiveWorkKind::Worker,
                    state,
                    status: status.to_string(),
                    label: format!("{name} · {}", agent.agent_type.as_str()),
                    detail: format!("{} · {}", agent.assignment.objective, agent.model),
                },
            );
        }
        let cached_worker_ids = app
            .subagent_cache
            .iter()
            .map(|agent| agent.agent_id.as_str())
            .collect::<std::collections::HashSet<_>>();
        for (agent_id, progress) in &app.agent_progress {
            if cached_worker_ids.contains(agent_id.as_str()) {
                continue;
            }
            let waiting = progress.to_ascii_lowercase().contains("waiting");
            insert_prefer_live(
                &mut by_identity,
                LiveWorkRow {
                    identity: format!("worker:{agent_id}"),
                    kind: LiveWorkKind::Worker,
                    state: if waiting {
                        LiveWorkState::Waiting
                    } else {
                        LiveWorkState::Active
                    },
                    status: if waiting { "waiting" } else { "running" }.to_string(),
                    label: app
                        .agent_label_map
                        .get(agent_id)
                        .cloned()
                        .unwrap_or_else(|| agent_id.clone()),
                    detail: progress.clone(),
                },
            );
        }

        if let Some(panel) = app.workflow_panel.as_ref() {
            let state = if panel.lifecycle.is_running() {
                LiveWorkState::Active
            } else {
                LiveWorkState::Settled
            };
            insert_prefer_live(
                &mut by_identity,
                LiveWorkRow {
                    identity: format!("workflow:{}", panel.run_id),
                    kind: LiveWorkKind::Workflow,
                    state,
                    status: panel.lifecycle.label().to_string(),
                    label: panel.label.clone(),
                    detail: panel.run_id.clone(),
                },
            );
        }

        let mut rows = by_identity.into_values().collect::<Vec<_>>();
        rows.sort_by(|left, right| {
            left.state
                .rank()
                .cmp(&right.state.rank())
                .then_with(|| left.identity.cmp(&right.identity))
        });
        let mut counts = LiveWorkCounts::default();
        for row in &rows {
            if row.state != LiveWorkState::Settled {
                counts.active += 1;
                match row.kind {
                    LiveWorkKind::Task => counts.tasks += 1,
                    LiveWorkKind::Run => counts.runs += 1,
                    LiveWorkKind::Worker => counts.workers += 1,
                    LiveWorkKind::Workflow => counts.workflows += 1,
                }
            }
        }
        Self { rows, counts }
    }
}

fn insert_prefer_live(rows: &mut HashMap<String, LiveWorkRow>, row: LiveWorkRow) {
    match rows.get(&row.identity) {
        Some(existing) if existing.state.rank() <= row.state.rank() => {}
        _ => {
            rows.insert(row.identity.clone(), row);
        }
    }
}

fn row_from_task(task: &TaskPanelEntry, kind: LiveWorkKind, identity: String) -> LiveWorkRow {
    let state = match task.status.as_str() {
        "waiting" | "needs_user" => LiveWorkState::Waiting,
        "running" | "queued" | "starting" => LiveWorkState::Active,
        _ => LiveWorkState::Settled,
    };
    LiveWorkRow {
        identity,
        kind,
        state,
        status: task.status.clone(),
        label: task.prompt_summary.clone(),
        detail: task.id.clone(),
    }
}

fn shell_id_from_task(task: &TaskPanelEntry) -> Option<String> {
    shell_id_from_text(&task.id).or_else(|| shell_id_from_text(&task.prompt_summary))
}

fn shell_id_from_text(text: &str) -> Option<String> {
    text.split(|c: char| c.is_whitespace() || c == ':' || c == '=' || c == '"')
        .find(|part| part.starts_with("shell_"))
        .map(str::to_string)
}

fn is_shell_wait_tool(name: &str) -> bool {
    matches!(name, "task_shell_wait" | "exec_shell_wait" | "exec_wait")
}

fn worker_state(
    status: Option<AgentWorkerStatus>,
    legacy: &SubAgentStatus,
    label: &str,
) -> LiveWorkState {
    if label == "waiting" {
        LiveWorkState::Waiting
    } else if status.is_some_and(|status| {
        matches!(
            status,
            AgentWorkerStatus::Completed
                | AgentWorkerStatus::Failed
                | AgentWorkerStatus::Cancelled
                | AgentWorkerStatus::Interrupted
        )
    }) || status.is_none() && !matches!(legacy, SubAgentStatus::Running)
    {
        LiveWorkState::Settled
    } else {
        LiveWorkState::Active
    }
}

fn worker_status(status: AgentWorkerStatus) -> &'static str {
    match status {
        AgentWorkerStatus::Queued => "queued",
        AgentWorkerStatus::Starting => "starting",
        AgentWorkerStatus::Running => "running",
        AgentWorkerStatus::WaitingForUser => "waiting",
        AgentWorkerStatus::ModelWait => "model wait",
        AgentWorkerStatus::RunningTool => "tool",
        AgentWorkerStatus::Completed => "done",
        AgentWorkerStatus::Failed => "failed",
        AgentWorkerStatus::Cancelled => "canceled",
        AgentWorkerStatus::Interrupted => "interrupted",
    }
}

fn subagent_status(status: &SubAgentStatus) -> &'static str {
    match status {
        SubAgentStatus::Running => "running",
        SubAgentStatus::Completed => "done",
        SubAgentStatus::Interrupted(_) => "interrupted",
        SubAgentStatus::Failed(_) => "failed",
        SubAgentStatus::Cancelled => "canceled",
        SubAgentStatus::BudgetExhausted => "budget",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_live_shell_runs_are_three_runs_not_one_task() {
        let entries = ["shell_a", "shell_b", "shell_c"]
            .into_iter()
            .map(|id| TaskPanelEntry {
                id: id.to_string(),
                status: "running".to_string(),
                prompt_summary: format!("shell: {id}"),
                duration_ms: None,
                kind: TaskPanelEntryKind::Background,
                stale: false,
                elapsed_since_output_ms: None,
                owner_agent_id: None,
                owner_agent_name: None,
            })
            .collect::<Vec<_>>();
        let mut by_identity = HashMap::new();
        for entry in &entries {
            insert_prefer_live(
                &mut by_identity,
                row_from_task(entry, LiveWorkKind::Run, format!("shell:{}", entry.id)),
            );
        }
        let rows = by_identity.into_values().collect::<Vec<_>>();
        let runs = rows
            .iter()
            .filter(|row| row.kind == LiveWorkKind::Run)
            .count();
        assert_eq!(runs, 3);
        assert_ne!(format!("Tasks {}", 1), format!("Runs {runs}"));
    }
}
