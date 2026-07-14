//! Transactional preview / commit / rollback / cancel callbacks.
//!
//! Theme, model, and provider pickers share the same lifecycle:
//! navigate → preview, Enter → commit, Esc → rollback (+ close), and an
//! explicit cancel hook for hosts that need a distinct abort path.

use std::borrow::Cow;

/// Host-facing transactional callbacks for a settings picker.
///
/// The framework invokes these with the selected option id (or nothing, for
/// rollback/cancel). Concrete modals typically translate these into
/// `ViewAction` / `ViewEvent` values rather than mutating `App` directly.
#[allow(dead_code)] // transactional layer for model/provider migration (TUI-DOG-009)
pub struct TransactionCallbacks<FPreview, FCommit, FRollback, FCancel>
where
    FPreview: FnMut(&str),
    FCommit: FnMut(&str),
    FRollback: FnMut(),
    FCancel: FnMut(),
{
    pub preview: FPreview,
    pub commit: FCommit,
    pub rollback: FRollback,
    pub cancel: FCancel,
}

impl<FPreview, FCommit, FRollback, FCancel>
    TransactionCallbacks<FPreview, FCommit, FRollback, FCancel>
where
    FPreview: FnMut(&str),
    FCommit: FnMut(&str),
    FRollback: FnMut(),
    FCancel: FnMut(),
{
    #[allow(dead_code)] // callback runners for host adapters (TUI-DOG-009)
    pub fn run_preview(&mut self, id: &str) {
        (self.preview)(id);
    }

    #[allow(dead_code)]
    pub fn run_commit(&mut self, id: &str) {
        (self.commit)(id);
    }

    #[allow(dead_code)]
    pub fn run_rollback(&mut self) {
        (self.rollback)();
    }

    #[allow(dead_code)]
    pub fn run_cancel(&mut self) {
        (self.cancel)();
    }
}

/// Recorded transactional events for tests and host adapters.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // event log for matrix tests + pull-style hosts (TUI-DOG-009)
pub enum TransactionEvent {
    Preview {
        id: Cow<'static, str>,
    },
    Commit {
        id: Cow<'static, str>,
    },
    Rollback,
    Cancel,
    ItemAction {
        option_id: Cow<'static, str>,
        action_id: Cow<'static, str>,
    },
}

/// Simple recorder used by matrix tests and by hosts that prefer pull-style
/// integration over closures.
#[derive(Debug, Default, Clone)]
#[allow(dead_code)] // pull-style transaction log (TUI-DOG-009)
pub struct TransactionLog {
    pub events: Vec<TransactionEvent>,
}

impl TransactionLog {
    #[allow(dead_code)]
    pub fn preview(&mut self, id: impl Into<Cow<'static, str>>) {
        self.events
            .push(TransactionEvent::Preview { id: id.into() });
    }

    #[allow(dead_code)]
    pub fn commit(&mut self, id: impl Into<Cow<'static, str>>) {
        self.events.push(TransactionEvent::Commit { id: id.into() });
    }

    #[allow(dead_code)]
    pub fn rollback(&mut self) {
        self.events.push(TransactionEvent::Rollback);
    }

    #[allow(dead_code)]
    pub fn cancel(&mut self) {
        self.events.push(TransactionEvent::Cancel);
    }

    #[allow(dead_code)]
    pub fn item_action(
        &mut self,
        option_id: impl Into<Cow<'static, str>>,
        action_id: impl Into<Cow<'static, str>>,
    ) {
        self.events.push(TransactionEvent::ItemAction {
            option_id: option_id.into(),
            action_id: action_id.into(),
        });
    }

    #[must_use]
    #[allow(dead_code)]
    pub fn last(&self) -> Option<&TransactionEvent> {
        self.events.last()
    }
}
