//! Active-session Work Graph authority and legacy tool adapters.

use std::sync::{Arc, Mutex, MutexGuard};

use crate::tools::plan::{PlanSnapshot, PlanState, SharedPlanState, StepStatus};
use crate::tools::todo::{SharedTodoList, TodoList, TodoListSnapshot, TodoStatus};

use super::{
    ApprovalRef, ChangeCtx, CompatPlanMetadata, CompatProjectionState, CompatTodoBinding, EdgeKind,
    NodeKind, NodeState, ProposalId, Provenance, WorkEdge, WorkEdgeId, WorkGraph, WorkGraphChange,
    WorkGraphProposal, WorkGraphSnapshot, WorkNode, WorkNodeId, WorkNodePatch, import_legacy,
    project_plan, project_todos, validate,
};

#[derive(Debug, Clone, PartialEq)]
pub struct WorkRuntimeSnapshot {
    pub graph: WorkGraphSnapshot,
    pub todos: TodoListSnapshot,
    pub plan: PlanSnapshot,
}

#[derive(Debug, Default)]
struct ActiveGraph {
    session_id: Option<String>,
    snapshot: Option<WorkGraphSnapshot>,
    pending_publish: bool,
}

/// One active session graph plus the read-only legacy views it publishes.
pub struct WorkRuntime {
    todos: SharedTodoList,
    plan: SharedPlanState,
    graph: Mutex<ActiveGraph>,
}

pub type SharedWorkRuntime = Arc<WorkRuntime>;

impl std::fmt::Debug for WorkRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let graph = lock_unpoisoned(&self.graph);
        f.debug_struct("WorkRuntime")
            .field("session_id", &graph.session_id)
            .field("has_graph", &graph.snapshot.is_some())
            .field("pending_publish", &graph.pending_publish)
            .finish()
    }
}

#[must_use]
pub fn new_shared_work_runtime(todos: SharedTodoList, plan: SharedPlanState) -> SharedWorkRuntime {
    Arc::new(WorkRuntime {
        todos,
        plan,
        graph: Mutex::new(ActiveGraph::default()),
    })
}

impl WorkRuntime {
    #[must_use]
    pub fn matches_todos(&self, todos: &SharedTodoList) -> bool {
        Arc::ptr_eq(&self.todos, todos)
    }

    #[must_use]
    pub fn matches_plan(&self, plan: &SharedPlanState) -> bool {
        Arc::ptr_eq(&self.plan, plan)
    }

    /// Apply an `update_plan` payload through the graph and publish both
    /// legacy projections only after the candidate graph validates.
    pub async fn apply_plan_update(
        &self,
        session_id: &str,
        tool: &str,
        plan: &PlanSnapshot,
    ) -> Result<PlanSnapshot, String> {
        let todos_guard = self.todos.lock().await;
        let plan_guard = self.plan.lock().await;
        let mut active = lock_unpoisoned(&self.graph);
        let base = graph_for_update(
            &mut active,
            session_id,
            &plan_guard.snapshot(),
            &todos_guard.snapshot(),
        )?;
        let next = update_plan_graph(base, session_id, tool, plan)?;
        let derived_plan = project_plan(&next);
        let derived_todos = project_todos(&next);
        let next_plan = PlanState::from_snapshot(&derived_plan);
        let next_todos = TodoList::from_snapshot(&derived_todos)?;
        validate_combined(&next, &next_plan.snapshot(), &next_todos.snapshot())?;
        active.snapshot = Some(next);
        active.pending_publish = true;
        Ok(derived_plan)
    }

    /// Apply a legacy To-do/checklist payload through the graph and publish
    /// both projections from the committed candidate.
    pub async fn apply_todo_update(
        &self,
        session_id: &str,
        tool: &str,
        todos: &TodoListSnapshot,
    ) -> Result<TodoListSnapshot, String> {
        let todos_guard = self.todos.lock().await;
        let plan_guard = self.plan.lock().await;
        let mut active = lock_unpoisoned(&self.graph);
        let base = graph_for_update(
            &mut active,
            session_id,
            &plan_guard.snapshot(),
            &todos_guard.snapshot(),
        )?;
        let next = update_todo_graph(base, session_id, tool, todos)?;
        let derived_plan = project_plan(&next);
        let derived_todos = project_todos(&next);
        let next_plan = PlanState::from_snapshot(&derived_plan);
        let next_todos = TodoList::from_snapshot(&derived_todos)?;
        validate_combined(&next, &next_plan.snapshot(), &next_todos.snapshot())?;
        active.snapshot = Some(next);
        active.pending_publish = true;
        Ok(derived_todos)
    }

    /// Alias every current plan step into the legacy To-do view. This
    /// replaces the old Plan→To-do bridge writer with a graph change.
    pub async fn accept_plan(
        &self,
        session_id: Option<&str>,
        approval_reference: &str,
    ) -> Result<usize, String> {
        let todos_guard = self.todos.lock().await;
        let plan_guard = self.plan.lock().await;
        let mut active = lock_unpoisoned(&self.graph);
        let session_id = resolved_session_id(&active, session_id);
        let base = graph_for_update(
            &mut active,
            &session_id,
            &plan_guard.snapshot(),
            &todos_guard.snapshot(),
        )?;
        if base.compat.plan_order.is_empty() {
            // There is no Plan diff to accept. A legacy-only To-do import may
            // still have happened while resolving the base and must receive
            // its first graph-bearing persistence boundary.
            if !base.is_empty() {
                active.snapshot = Some(base);
                active.pending_publish = true;
            }
            return Ok(0);
        }
        let mut graph = WorkGraph::from_snapshot(base);
        let active_plan_node = graph
            .snapshot()
            .compat
            .plan_order
            .iter()
            .find(|id| {
                graph
                    .snapshot()
                    .node(id)
                    .is_some_and(|node| node.state == NodeState::Active)
            })
            .cloned();
        if let Some(active_plan_node) = active_plan_node.as_ref() {
            let competing = graph
                .snapshot()
                .compat
                .todos
                .iter()
                .filter(|binding| &binding.node != active_plan_node)
                .filter_map(|binding| {
                    graph
                        .snapshot()
                        .node(&binding.node)
                        .filter(|node| node.state == NodeState::Active)
                        .map(|node| (node.id.clone(), node.title.clone()))
                })
                .collect::<Vec<_>>();
            for (id, title) in competing {
                patch_existing_node(
                    &mut graph,
                    &session_id,
                    "plan_acceptance",
                    &id,
                    title,
                    NodeState::Ready,
                )?;
            }
        }
        let mut compat = graph.snapshot().compat.clone();
        let mut next_id = compat
            .todos
            .iter()
            .map(|binding| binding.legacy_id)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| "To-do item IDs are exhausted".to_string())?;
        let before = compat.todos.len();
        for (index, node) in compat.plan_order.iter().enumerate() {
            if compat.todos.iter().any(|binding| &binding.node == node) {
                continue;
            }
            compat.todos.push(CompatTodoBinding {
                legacy_id: next_id,
                node: node.clone(),
                plan_index: Some(u32::try_from(index).map_err(|_| "too many plan steps")?),
            });
            next_id = next_id
                .checked_add(1)
                .ok_or_else(|| "To-do item IDs are exhausted".to_string())?;
        }
        let changed = compat.todos.len().saturating_sub(before);
        let proposal_id = ProposalId::derive(
            &session_id,
            &format!("plan-acceptance:{}", graph.snapshot().revision),
        );
        apply_change(
            &mut graph,
            &session_id,
            "plan_acceptance",
            WorkGraphChange::ProposePlanDiff {
                proposal: WorkGraphProposal {
                    id: proposal_id.clone(),
                    added_nodes: Vec::new(),
                    added_edges: Vec::new(),
                    updated_nodes: Vec::new(),
                    removed_edges: Vec::new(),
                },
            },
        )?;
        apply_change(
            &mut graph,
            &session_id,
            "plan_acceptance",
            WorkGraphChange::AcceptPlanDiff {
                proposal_id,
                approval: ApprovalRef {
                    reference: approval_reference.to_string(),
                },
            },
        )?;
        apply_change(
            &mut graph,
            &session_id,
            "plan_acceptance",
            WorkGraphChange::ReplaceCompatProjection { compat },
        )?;
        let next = graph.into_snapshot();
        let derived_plan = project_plan(&next);
        let derived_todos = project_todos(&next);
        let next_plan = PlanState::from_snapshot(&derived_plan);
        let next_todos = TodoList::from_snapshot(&derived_todos)?;
        validate_combined(&next, &next_plan.snapshot(), &next_todos.snapshot())?;
        active.snapshot = Some(next);
        active.pending_publish = true;
        Ok(changed)
    }

    /// Publish the latest validated legacy views after the caller has queued
    /// their graph-backed session/checkpoint write.
    pub async fn publish_pending(&self) -> Result<bool, String> {
        let mut todos = self.todos.lock().await;
        let mut plan = self.plan.lock().await;
        let mut active = lock_unpoisoned(&self.graph);
        if !active.pending_publish {
            return Ok(false);
        }
        let graph = active
            .snapshot
            .as_ref()
            .ok_or_else(|| "pending Work projection has no graph".to_string())?;
        let derived_plan = project_plan(graph);
        let derived_todos = project_todos(graph);
        let next_plan = PlanState::from_snapshot(&derived_plan);
        let next_todos = TodoList::from_snapshot(&derived_todos)?;
        validate_combined(graph, &next_plan.snapshot(), &next_todos.snapshot())?;
        *plan = next_plan;
        *todos = next_todos;
        active.pending_publish = false;
        Ok(true)
    }

    /// Synchronous counterpart for explicit save/rename/fork commands that
    /// have already completed their atomic disk write.
    pub fn publish_pending_sync(&self) -> Result<bool, String> {
        let mut todos = retry_lock(&self.todos, 100).ok_or_else(|| {
            "To-do state is busy; saved Work views were not published".to_string()
        })?;
        let mut plan = retry_lock(&self.plan, 100)
            .ok_or_else(|| "Plan state is busy; saved Work views were not published".to_string())?;
        let mut active = lock_unpoisoned(&self.graph);
        if !active.pending_publish {
            return Ok(false);
        }
        let graph = active
            .snapshot
            .as_ref()
            .ok_or_else(|| "pending Work projection has no graph".to_string())?;
        let derived_plan = project_plan(graph);
        let derived_todos = project_todos(graph);
        let next_plan = PlanState::from_snapshot(&derived_plan);
        let next_todos = TodoList::from_snapshot(&derived_todos)?;
        validate_combined(graph, &next_plan.snapshot(), &next_todos.snapshot())?;
        *plan = next_plan;
        *todos = next_todos;
        active.pending_publish = false;
        Ok(true)
    }

    #[must_use]
    pub fn has_pending_publish(&self) -> bool {
        lock_unpoisoned(&self.graph).pending_publish
    }

    /// Latest graph-derived To-do view, including an unpublished transaction.
    pub async fn current_todos(&self) -> Result<TodoListSnapshot, String> {
        let projected = {
            let active = lock_unpoisoned(&self.graph);
            active.snapshot.as_ref().map(project_todos)
        };
        if let Some(projected) = projected {
            return Ok(projected);
        }
        Ok(self.todos.lock().await.snapshot())
    }

    /// Capture a persistence-ready graph plus fully populated old views.
    /// Legacy-only in-memory state is imported once and normalized in place.
    pub fn capture(&self, session_id: Option<&str>) -> Result<Option<WorkRuntimeSnapshot>, String> {
        self.capture_with_retries(session_id, 100)
    }

    /// Non-blocking capture for the render/event loop.
    pub fn try_capture(
        &self,
        session_id: Option<&str>,
    ) -> Result<Option<WorkRuntimeSnapshot>, String> {
        self.capture_with_retries(session_id, 1)
    }

    fn capture_with_retries(
        &self,
        session_id: Option<&str>,
        retries: u32,
    ) -> Result<Option<WorkRuntimeSnapshot>, String> {
        let todos = retry_lock(&self.todos, retries)
            .ok_or_else(|| "To-do state is busy; try saving again".to_string())?;
        let plan = retry_lock(&self.plan, retries)
            .ok_or_else(|| "Plan state is busy; try saving again".to_string())?;
        let mut active = lock_unpoisoned(&self.graph);
        let todos_snapshot = todos.snapshot();
        let plan_snapshot = plan.snapshot();
        if todos_snapshot.is_empty()
            && plan_snapshot.is_empty()
            && active
                .snapshot
                .as_ref()
                .is_none_or(WorkGraphSnapshot::is_empty)
        {
            return Ok(None);
        }
        let had_graph = active.snapshot.is_some();
        let had_pending_publish = active.pending_publish;
        let session_id = resolved_session_id(&active, session_id);
        let graph = graph_for_update(&mut active, &session_id, &plan_snapshot, &todos_snapshot)?;
        let derived_plan = project_plan(&graph);
        let derived_todos = project_todos(&graph);
        validate_combined(&graph, &derived_plan, &derived_todos)?;
        if had_graph
            && !had_pending_publish
            && (derived_plan != plan_snapshot || derived_todos != todos_snapshot)
        {
            return Err("live Work Graph and legacy views disagree".to_string());
        }
        active.snapshot = Some(graph.clone());
        if !had_graph {
            active.pending_publish = true;
        }
        Ok(Some(WorkRuntimeSnapshot {
            graph,
            todos: derived_todos,
            plan: derived_plan,
        }))
    }

    /// Validate and atomically activate persisted state. Sessions without a
    /// graph are deterministically imported from their complete old views.
    pub fn restore(
        &self,
        session_id: &str,
        graph: Option<&WorkGraphSnapshot>,
        todos: &TodoListSnapshot,
        plan: &PlanSnapshot,
    ) -> Result<Option<WorkRuntimeSnapshot>, String> {
        let had_graph = graph.is_some();
        let graph = match graph {
            Some(graph) => {
                validate(graph).map_err(|err| err.to_string())?;
                graph.clone()
            }
            None if todos.is_empty() && plan.is_empty() => WorkGraphSnapshot::new(),
            None => import_legacy(session_id, plan, todos)?,
        };
        let derived_plan = project_plan(&graph);
        let derived_todos = project_todos(&graph);
        if graph.is_empty() {
            if !todos.is_empty() || !plan.is_empty() {
                return Err("empty Work Graph cannot carry non-empty legacy views".to_string());
            }
        } else if graph.import_digest.is_some() && graph.compat.is_empty() {
            return Err("imported Work Graph is missing compatibility projections".to_string());
        }
        validate_combined(&graph, &derived_plan, &derived_todos)?;
        if had_graph && (&derived_plan != plan || &derived_todos != todos) {
            return Err("persisted Work Graph and legacy views disagree".to_string());
        }
        let next_plan = PlanState::from_snapshot(&derived_plan);
        let next_todos = TodoList::from_snapshot(&derived_todos)?;
        let mut todos_guard = retry_lock(&self.todos, 100)
            .ok_or_else(|| "To-do state is busy; session was not restored".to_string())?;
        let mut plan_guard = retry_lock(&self.plan, 100)
            .ok_or_else(|| "Plan state is busy; session was not restored".to_string())?;
        let mut active = lock_unpoisoned(&self.graph);
        *todos_guard = next_todos;
        *plan_guard = next_plan;
        active.session_id = Some(session_id.to_string());
        active.snapshot = Some(graph.clone());
        // A legacy load has already restored its complete old views, but its
        // newly imported graph still needs one acknowledged graph-bearing
        // write (and pre-import archive) before the migration is settled.
        active.pending_publish = !had_graph && !graph.is_empty();
        if graph.is_empty() {
            Ok(None)
        } else {
            Ok(Some(WorkRuntimeSnapshot {
                graph,
                todos: derived_todos,
                plan: derived_plan,
            }))
        }
    }

    pub fn clear(&self, session_id: Option<&str>) -> bool {
        let Some(mut todos) = retry_lock(&self.todos, 100) else {
            return false;
        };
        let Some(mut plan) = retry_lock(&self.plan, 100) else {
            return false;
        };
        let mut active = lock_unpoisoned(&self.graph);
        todos.clear();
        *plan = PlanState::default();
        active.session_id = Some(resolved_session_id(&active, session_id));
        active.snapshot = Some(WorkGraphSnapshot::new());
        active.pending_publish = false;
        true
    }
}

fn graph_for_update(
    active: &mut ActiveGraph,
    session_id: &str,
    plan: &PlanSnapshot,
    todos: &TodoListSnapshot,
) -> Result<WorkGraphSnapshot, String> {
    match active.session_id.as_deref() {
        // App session transitions are already blocked while runtime work is
        // active. Rebind the authority namespace without re-keying graph IDs
        // so save-as/fork/new-session flows keep one coherent snapshot.
        Some(active_id) if active_id != session_id => {
            active.session_id = Some(session_id.to_string());
        }
        None => active.session_id = Some(session_id.to_string()),
        Some(_) => {}
    }
    if let Some(snapshot) = active.snapshot.as_ref() {
        validate(snapshot).map_err(|err| err.to_string())?;
        return Ok(snapshot.clone());
    }
    let graph = if plan.is_empty() && todos.is_empty() {
        WorkGraphSnapshot::new()
    } else {
        import_legacy(session_id, plan, todos)?
    };
    active.snapshot = Some(graph.clone());
    Ok(graph)
}

fn update_plan_graph(
    base: WorkGraphSnapshot,
    session_id: &str,
    tool: &str,
    plan: &PlanSnapshot,
) -> Result<WorkGraphSnapshot, String> {
    let mut graph = WorkGraph::from_snapshot(base);
    let objective = ensure_objective(&mut graph, session_id, tool, plan)?;
    let desired_active_alias = plan.items.iter().enumerate().find_map(|(index, item)| {
        (item.status == StepStatus::InProgress)
            .then(|| graph.snapshot().compat.plan_order.get(index).cloned())
            .flatten()
            .filter(|node| {
                graph
                    .snapshot()
                    .compat
                    .todos
                    .iter()
                    .any(|binding| &binding.node == node)
            })
    });
    if desired_active_alias.is_some() {
        deactivate_projected_todos(&mut graph, session_id, tool)?;
    }

    let mut order = Vec::with_capacity(plan.items.len());
    for (index, item) in plan.items.iter().enumerate() {
        let id = graph
            .snapshot()
            .compat
            .plan_order
            .get(index)
            .cloned()
            .unwrap_or_else(|| WorkNodeId::derive(session_id, &format!("plan:{index}")));
        let provenance = tool_provenance(graph.snapshot(), tool);
        upsert_node(
            &mut graph,
            session_id,
            tool,
            WorkNode {
                id: id.clone(),
                kind: NodeKind::PlanStep,
                title: item.step.trim().to_string(),
                state: plan_node_state(&item.status),
                acceptance: Vec::new(),
                binding: None,
                evidence: None,
                provenance,
                created_at: now_ms(),
                updated_at: now_ms(),
            },
        )?;
        ensure_contains(&mut graph, session_id, tool, &objective, &id)?;
        order.push(id);
    }
    let mut compat = graph.snapshot().compat.clone();
    compat.plan = CompatPlanMetadata::from_plan_snapshot(plan);
    compat.plan_order = order;
    compat.todos.retain(|binding| {
        binding.plan_index.is_none_or(|index| {
            usize::try_from(index)
                .ok()
                .is_some_and(|i| i < plan.items.len())
        })
    });
    for binding in &mut compat.todos {
        if let Some(index) = binding.plan_index
            && let Some(node) = compat
                .plan_order
                .get(usize::try_from(index).unwrap_or(usize::MAX))
        {
            binding.node.clone_from(node);
        }
    }
    apply_change(
        &mut graph,
        session_id,
        tool,
        WorkGraphChange::ReplaceCompatProjection { compat },
    )?;
    Ok(graph.into_snapshot())
}

fn update_todo_graph(
    base: WorkGraphSnapshot,
    session_id: &str,
    tool: &str,
    todos: &TodoListSnapshot,
) -> Result<WorkGraphSnapshot, String> {
    let mut graph = WorkGraph::from_snapshot(base);
    deactivate_projected_todos(&mut graph, session_id, tool)?;
    let current_plan = project_plan(graph.snapshot());
    let objective = ensure_objective(&mut graph, session_id, tool, &current_plan)?;
    let plan_order = graph.snapshot().compat.plan_order.clone();
    let mut bindings = Vec::with_capacity(todos.items.len());
    for item in &todos.items {
        let title = item.content.trim().to_string();
        let alias = graph
            .snapshot()
            .compat
            .todos
            .iter()
            .find(|binding| binding.legacy_id == item.id)
            .and_then(|binding| {
                binding
                    .plan_index
                    .map(|index| (index, binding.node.clone()))
            })
            .filter(|(index, node)| {
                plan_order.get(usize::try_from(*index).unwrap_or(usize::MAX)) == Some(node)
            });
        let (node, plan_index) = if let Some((index, node)) = alias {
            patch_existing_node(
                &mut graph,
                session_id,
                tool,
                &node,
                title,
                todo_node_state(item.status),
            )?;
            (node, Some(index))
        } else {
            let node = graph
                .snapshot()
                .compat
                .todos
                .iter()
                .find(|binding| binding.legacy_id == item.id && binding.plan_index.is_none())
                .map(|binding| binding.node.clone())
                .unwrap_or_else(|| WorkNodeId::derive(session_id, &format!("todo:{}", item.id)));
            let desired = todo_node_state(item.status);
            let provenance = tool_provenance(graph.snapshot(), tool);
            upsert_node(
                &mut graph,
                session_id,
                tool,
                WorkNode {
                    id: node.clone(),
                    kind: NodeKind::PlanStep,
                    title,
                    state: if desired == NodeState::Active {
                        NodeState::Ready
                    } else {
                        desired
                    },
                    acceptance: Vec::new(),
                    binding: None,
                    evidence: None,
                    provenance,
                    created_at: now_ms(),
                    updated_at: now_ms(),
                },
            )?;
            ensure_contains(&mut graph, session_id, tool, &objective, &node)?;
            if desired == NodeState::Active {
                let clean_title = graph
                    .snapshot()
                    .node(&node)
                    .map(|node| node.title.clone())
                    .ok_or_else(|| format!("node {node} not found after insert"))?;
                patch_existing_node(&mut graph, session_id, tool, &node, clean_title, desired)?;
            }
            (node, None)
        };
        bindings.push(CompatTodoBinding {
            legacy_id: item.id,
            node,
            plan_index,
        });
    }
    let mut compat = graph.snapshot().compat.clone();
    compat.todos = bindings;
    apply_change(
        &mut graph,
        session_id,
        tool,
        WorkGraphChange::ReplaceCompatProjection { compat },
    )?;
    Ok(graph.into_snapshot())
}

fn ensure_objective(
    graph: &mut WorkGraph,
    session_id: &str,
    tool: &str,
    plan: &PlanSnapshot,
) -> Result<WorkNodeId, String> {
    let id = graph
        .snapshot()
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::Objective)
        .map(|node| node.id.clone())
        .unwrap_or_else(|| WorkNodeId::derive(session_id, "objective"));
    let title = plan
        .objective
        .as_deref()
        .or(plan.title.as_deref())
        .unwrap_or("Session work")
        .to_string();
    upsert_node(
        graph,
        session_id,
        tool,
        WorkNode {
            id: id.clone(),
            kind: NodeKind::Objective,
            title,
            state: NodeState::Ready,
            acceptance: Vec::new(),
            binding: None,
            evidence: None,
            provenance: tool_provenance(graph.snapshot(), tool),
            created_at: now_ms(),
            updated_at: now_ms(),
        },
    )?;
    Ok(id)
}

fn upsert_node(
    graph: &mut WorkGraph,
    session_id: &str,
    tool: &str,
    node: WorkNode,
) -> Result<(), String> {
    if let Some(existing) = graph.snapshot().node(&node.id) {
        if existing.kind != node.kind {
            return Err(format!("node {} changed kind", node.id));
        }
        patch_existing_node(graph, session_id, tool, &node.id, node.title, node.state)
    } else {
        apply_change(graph, session_id, tool, WorkGraphChange::AddNode { node })
    }
}

fn patch_existing_node(
    graph: &mut WorkGraph,
    session_id: &str,
    tool: &str,
    id: &WorkNodeId,
    title: String,
    state: NodeState,
) -> Result<(), String> {
    let current = graph
        .snapshot()
        .node(id)
        .ok_or_else(|| format!("node {id} not found"))?;
    let provenance = tool_provenance(graph.snapshot(), tool);
    if current.title == title && current.state == state && current.provenance == provenance {
        return Ok(());
    }
    apply_change(
        graph,
        session_id,
        tool,
        WorkGraphChange::UpdateNode {
            id: id.clone(),
            patch: WorkNodePatch {
                title: Some(title),
                state: Some(state),
                provenance: Some(provenance),
                ..WorkNodePatch::default()
            },
        },
    )
}

fn ensure_contains(
    graph: &mut WorkGraph,
    session_id: &str,
    tool: &str,
    parent: &WorkNodeId,
    child: &WorkNodeId,
) -> Result<(), String> {
    let id = WorkEdgeId::derive(
        session_id,
        &format!("contains:{}:{}", parent.as_str(), child.as_str()),
    );
    if graph.snapshot().edge(&id).is_some() {
        return Ok(());
    }
    apply_change(
        graph,
        session_id,
        tool,
        WorkGraphChange::AddEdge {
            edge: WorkEdge {
                id,
                kind: EdgeKind::Contains,
                from: parent.clone(),
                to: child.clone(),
            },
        },
    )
}

fn deactivate_projected_todos(
    graph: &mut WorkGraph,
    session_id: &str,
    tool: &str,
) -> Result<(), String> {
    let active = graph
        .snapshot()
        .compat
        .todos
        .iter()
        .filter_map(|binding| {
            graph
                .snapshot()
                .node(&binding.node)
                .filter(|node| node.state == NodeState::Active)
                .map(|node| (node.id.clone(), node.title.clone()))
        })
        .collect::<Vec<_>>();
    for (id, title) in active {
        patch_existing_node(graph, session_id, tool, &id, title, NodeState::Ready)?;
    }
    Ok(())
}

fn apply_change(
    graph: &mut WorkGraph,
    session_id: &str,
    tool: &str,
    change: WorkGraphChange,
) -> Result<(), String> {
    graph
        .apply(
            change,
            ChangeCtx {
                session_id: session_id.to_string(),
                now: now_ms(),
                idempotency_key: None,
            },
        )
        .map(|_| ())
        .map_err(|err| format!("{tool}: {err}"))
}

fn validate_combined(
    graph: &WorkGraphSnapshot,
    plan: &PlanSnapshot,
    todos: &TodoListSnapshot,
) -> Result<(), String> {
    validate(graph).map_err(|err| err.to_string())?;
    if &project_plan(graph) != plan {
        return Err("Work Graph Plan projection is inconsistent".to_string());
    }
    if &project_todos(graph) != todos {
        return Err("Work Graph To-do projection is inconsistent".to_string());
    }
    TodoList::from_snapshot(todos)?;
    Ok(())
}

fn tool_provenance(snapshot: &WorkGraphSnapshot, tool: &str) -> Provenance {
    Provenance::ToolUpdate {
        tool: tool.to_string(),
        call_id: format!("{tool}:{}", snapshot.revision.saturating_add(1)),
    }
}

fn plan_node_state(status: &StepStatus) -> NodeState {
    match status {
        StepStatus::Pending => NodeState::Ready,
        StepStatus::InProgress => NodeState::Active,
        StepStatus::Completed => NodeState::Completed,
    }
}

fn todo_node_state(status: TodoStatus) -> NodeState {
    match status {
        TodoStatus::Pending => NodeState::Ready,
        TodoStatus::InProgress => NodeState::Active,
        TodoStatus::Completed => NodeState::Completed,
    }
}

fn resolved_session_id(active: &ActiveGraph, requested: Option<&str>) -> String {
    requested
        .map(str::to_string)
        .or_else(|| active.session_id.clone())
        .unwrap_or_else(|| "unsaved-work".to_string())
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn retry_lock<T>(
    mutex: &tokio::sync::Mutex<T>,
    retries: u32,
) -> Option<tokio::sync::MutexGuard<'_, T>> {
    for _ in 0..retries {
        if let Ok(guard) = mutex.try_lock() {
            return Some(guard);
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::plan::PlanItemArg;
    use crate::tools::todo::TodoItem;

    #[tokio::test]
    async fn adapters_keep_graph_and_both_legacy_views_in_lockstep() {
        let todos = crate::tools::todo::new_shared_todo_list();
        let plan = crate::tools::plan::new_shared_plan_state();
        let runtime = new_shared_work_runtime(todos.clone(), plan.clone());
        let plan_snapshot = PlanSnapshot {
            objective: Some("Release".to_string()),
            items: vec![PlanItemArg {
                step: "Test".to_string(),
                status: StepStatus::InProgress,
            }],
            ..PlanSnapshot::default()
        };
        runtime
            .apply_plan_update("session", "update_plan", &plan_snapshot)
            .await
            .expect("plan update");
        assert_eq!(
            runtime.accept_plan(Some("session"), "accept_act").await,
            Ok(1)
        );
        assert_eq!(runtime.publish_pending().await, Ok(true));
        assert_eq!(
            runtime.accept_plan(Some("session"), "accept_act").await,
            Ok(0),
            "already-aliased acceptance adds no duplicate To-do rows"
        );
        let repeated_acceptance = runtime
            .capture(Some("session"))
            .expect("capture repeated acceptance")
            .expect("state");
        assert_eq!(
            repeated_acceptance
                .graph
                .nodes
                .iter()
                .filter(|node| node.kind == NodeKind::Approval)
                .count(),
            2,
            "every explicit acceptance retains its own Approval node"
        );
        assert_eq!(runtime.publish_pending().await, Ok(true));
        let todo_snapshot = todos.lock().await.snapshot();
        assert_eq!(todo_snapshot.items.len(), 1);
        let desired = TodoListSnapshot {
            items: vec![TodoItem {
                id: todo_snapshot.items[0].id,
                content: todo_snapshot.items[0].content.clone(),
                status: TodoStatus::Completed,
            }],
            completion_pct: 100,
            in_progress_id: None,
        };
        runtime
            .apply_todo_update("session", "work_update", &desired)
            .await
            .expect("todo update");
        assert_eq!(runtime.publish_pending().await, Ok(true));
        assert_eq!(
            plan.lock().await.snapshot().items[0].status,
            StepStatus::Completed
        );
        let captured = runtime
            .capture(Some("session"))
            .expect("capture")
            .expect("state");
        assert_eq!(project_plan(&captured.graph), captured.plan);
        assert_eq!(project_todos(&captured.graph), captured.todos);
    }

    #[tokio::test]
    async fn accepting_active_plan_step_preserves_single_active_todo() {
        let todos = crate::tools::todo::new_shared_todo_list();
        let plan = crate::tools::plan::new_shared_plan_state();
        let runtime = new_shared_work_runtime(todos, plan);
        runtime
            .apply_todo_update(
                "session",
                "work_update",
                &TodoListSnapshot {
                    items: vec![TodoItem {
                        id: 1,
                        content: "Existing work".to_string(),
                        status: TodoStatus::InProgress,
                    }],
                    completion_pct: 0,
                    in_progress_id: Some(1),
                },
            )
            .await
            .expect("todo update");
        runtime
            .apply_plan_update(
                "session",
                "update_plan",
                &PlanSnapshot {
                    items: vec![PlanItemArg {
                        step: "Accepted work".to_string(),
                        status: StepStatus::InProgress,
                    }],
                    ..PlanSnapshot::default()
                },
            )
            .await
            .expect("plan update");
        assert_eq!(
            runtime
                .capture(Some("session"))
                .expect("capture")
                .expect("state")
                .todos
                .in_progress_id,
            Some(1),
            "an unaccepted plan must not disturb active To-do work"
        );

        runtime
            .accept_plan(Some("session"), "accept_act")
            .await
            .expect("accept plan");
        let captured = runtime
            .capture(Some("session"))
            .expect("capture")
            .expect("state");
        assert_eq!(
            captured
                .todos
                .items
                .iter()
                .filter(|item| item.status == TodoStatus::InProgress)
                .count(),
            1
        );
        assert_eq!(captured.todos.in_progress_id, Some(2));
        assert!(
            captured
                .graph
                .nodes
                .iter()
                .any(|node| node.kind == NodeKind::Approval),
            "accepted plans must retain an Approval node"
        );
    }

    #[test]
    fn legacy_restore_stays_pending_until_first_graph_bearing_write() {
        let todos = crate::tools::todo::new_shared_todo_list();
        let plan = crate::tools::plan::new_shared_plan_state();
        let runtime = new_shared_work_runtime(todos.clone(), plan.clone());
        let legacy_plan = PlanSnapshot {
            items: vec![PlanItemArg {
                step: "Migrate once".to_string(),
                status: StepStatus::InProgress,
            }],
            ..PlanSnapshot::default()
        };

        runtime
            .restore(
                "legacy-session",
                None,
                &TodoListSnapshot::default(),
                &legacy_plan,
            )
            .expect("restore legacy state");
        assert!(runtime.has_pending_publish());
        let captured = runtime
            .capture(Some("legacy-session"))
            .expect("capture imported graph")
            .expect("state");
        assert!(captured.graph.import_digest.is_some());
        assert_eq!(plan.blocking_lock().snapshot(), legacy_plan);
        assert_eq!(runtime.publish_pending_sync(), Ok(true));
        assert!(!runtime.has_pending_publish());
        assert!(todos.blocking_lock().snapshot().is_empty());
    }
}
