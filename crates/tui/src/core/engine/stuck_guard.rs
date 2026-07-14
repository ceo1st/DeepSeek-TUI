//! Conservative structural detection for model turns that make no progress.

use std::collections::VecDeque;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

const REPEAT_WARN_THRESHOLD: usize = 3;
const ALTERNATION_WARN_THRESHOLD: usize = 1;
const NO_PROGRESS_WARN_THRESHOLD: usize = 4;
const REPEATS_AFTER_WARN_TO_STOP: usize = 2;
const ALTERNATION_HISTORY: usize = 4;

/// A compact, semantic description of one completed model step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum StepFingerprint {
    Tool {
        name: String,
        arguments_hash: u64,
        error_signature: Option<u64>,
    },
    AssistantNoTool {
        text_hash: u64,
    },
}

impl StepFingerprint {
    pub(super) fn tool(
        name: impl Into<String>,
        arguments: &serde_json::Value,
        error: Option<&str>,
    ) -> Self {
        Self::Tool {
            name: name.into(),
            arguments_hash: stable_hash(&canonical_json(arguments).to_string()),
            error_signature: error.map(normalized_text_hash),
        }
    }

    pub(super) fn assistant_no_tool(text: &str) -> Self {
        Self::AssistantNoTool {
            text_hash: normalized_text_hash(text),
        }
    }
}

/// Signal emitted by [`StuckGuard::observe`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StuckSignal {
    Warn,
    Stop,
}

/// Per-turn detector. A change in the fingerprint resets the active episode,
/// so legitimate repeated tool names with different arguments are progress.
#[derive(Debug, Default)]
pub(super) struct StuckGuard {
    last_step: Option<StepFingerprint>,
    last_tool_action: Option<(String, u64)>,
    repeated_actions: usize,
    repeated_pairs: usize,
    no_progress_messages: usize,
    tool_history: VecDeque<StepFingerprint>,
    alternation_repeats: usize,
    warned: bool,
    repeats_after_warning: usize,
}

impl StuckGuard {
    pub(super) fn observe(&mut self, step: StepFingerprint) -> Option<StuckSignal> {
        match step {
            StepFingerprint::AssistantNoTool { .. } => self.observe_assistant(step),
            StepFingerprint::Tool { .. } => self.observe_tool(step),
        }
    }

    fn observe_assistant(&mut self, step: StepFingerprint) -> Option<StuckSignal> {
        self.tool_history.clear();
        self.alternation_repeats = 0;
        if self.last_step.as_ref() == Some(&step) {
            self.no_progress_messages = self.no_progress_messages.saturating_add(1);
        } else {
            self.reset_episode();
            self.last_step = Some(step);
            self.no_progress_messages = 1;
        }
        if self.no_progress_messages >= NO_PROGRESS_WARN_THRESHOLD {
            return self.signal_for_repeat();
        }
        None
    }

    fn observe_tool(&mut self, step: StepFingerprint) -> Option<StuckSignal> {
        self.no_progress_messages = 0;
        let action = match &step {
            StepFingerprint::Tool {
                name,
                arguments_hash,
                ..
            } => (name.clone(), *arguments_hash),
            StepFingerprint::AssistantNoTool { .. } => unreachable!(),
        };
        let same_action = self.last_tool_action.as_ref() == Some(&action);
        let same_pair = self.last_step.as_ref() == Some(&step);
        if same_action {
            self.repeated_actions = self.repeated_actions.saturating_add(1);
        } else {
            self.last_tool_action = Some(action);
            self.repeated_actions = 1;
        }
        self.repeated_pairs = if same_pair {
            self.repeated_pairs.saturating_add(1)
        } else {
            1
        };
        self.last_step = Some(step.clone());

        self.tool_history.push_back(step);
        while self.tool_history.len() > ALTERNATION_HISTORY {
            self.tool_history.pop_front();
        }
        if self.tool_history.len() == ALTERNATION_HISTORY {
            let history: Vec<_> = self.tool_history.iter().collect();
            if history[0] == history[2] && history[1] == history[3] && history[0] != history[1] {
                self.alternation_repeats = self.alternation_repeats.saturating_add(1);
            } else if !same_action {
                self.alternation_repeats = 0;
                self.warned = false;
                self.repeats_after_warning = 0;
            }
        }

        if self.repeated_actions >= REPEAT_WARN_THRESHOLD
            || self.repeated_pairs >= REPEAT_WARN_THRESHOLD
            || self.alternation_repeats >= ALTERNATION_WARN_THRESHOLD
        {
            return self.signal_for_repeat();
        }
        None
    }

    fn signal_for_repeat(&mut self) -> Option<StuckSignal> {
        if !self.warned {
            self.warned = true;
            self.repeats_after_warning = 0;
            Some(StuckSignal::Warn)
        } else {
            self.repeats_after_warning = self.repeats_after_warning.saturating_add(1);
            (self.repeats_after_warning >= REPEATS_AFTER_WARN_TO_STOP).then_some(StuckSignal::Stop)
        }
    }

    fn reset_episode(&mut self) {
        self.last_tool_action = None;
        self.repeated_actions = 0;
        self.repeated_pairs = 0;
        self.no_progress_messages = 0;
        self.tool_history.clear();
        self.alternation_repeats = 0;
        self.warned = false;
        self.repeats_after_warning = 0;
    }
}

pub(super) const RUNTIME_NOTICE: &str = "<codewhale:runtime_event kind=\"stuck_guard\" visibility=\"internal\">\n\
This is an internal runtime event. The previous steps appear to be repeating without progress.\n\
Change strategy: vary the tool arguments or method, inspect the latest result, or ask for the\n\
missing information. Do not repeat the same action unchanged.\n\
</codewhale:runtime_event>";

fn normalized_text_hash(text: &str) -> u64 {
    stable_hash(&text.split_whitespace().collect::<Vec<_>>().join(" "))
}

fn stable_hash(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

fn canonical_json(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(object) => {
            let mut entries: Vec<_> = object.iter().collect();
            entries.sort_by_key(|(key, _)| *key);
            let mut canonical = serde_json::Map::new();
            for (key, value) in entries {
                canonical.insert(key.clone(), canonical_json(value));
            }
            serde_json::Value::Object(canonical)
        }
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.iter().map(canonical_json).collect())
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool(name: &str, args: serde_json::Value) -> StepFingerprint {
        StepFingerprint::tool(name, &args, None)
    }

    fn failed_tool(name: &str, args: serde_json::Value, error: &str) -> StepFingerprint {
        StepFingerprint::tool(name, &args, Some(error))
    }

    #[test]
    fn identical_actions_warn_then_stop() {
        let step = tool("read_file", json!({"path": "a.txt"}));
        let mut guard = StuckGuard::default();
        assert_eq!(guard.observe(step.clone()), None);
        assert_eq!(guard.observe(step.clone()), None);
        assert_eq!(guard.observe(step.clone()), Some(StuckSignal::Warn));
        assert_eq!(guard.observe(step.clone()), None);
        assert_eq!(guard.observe(step.clone()), Some(StuckSignal::Stop));
        assert_eq!(guard.observe(step), Some(StuckSignal::Stop));
    }

    #[test]
    fn identical_action_error_pairs_are_detected() {
        let step = failed_tool("exec_shell", json!({"command": "missing"}), "not found");
        let mut guard = StuckGuard::default();
        assert_eq!(guard.observe(step.clone()), None);
        assert_eq!(guard.observe(step.clone()), None);
        assert_eq!(guard.observe(step), Some(StuckSignal::Warn));
    }

    #[test]
    fn identical_actions_with_different_errors_are_detected_too() {
        let args = json!({"command": "missing"});
        let mut guard = StuckGuard::default();
        assert_eq!(
            guard.observe(failed_tool("exec_shell", args.clone(), "not found")),
            None
        );
        assert_eq!(
            guard.observe(failed_tool("exec_shell", args.clone(), "still missing")),
            None
        );
        assert_eq!(
            guard.observe(failed_tool("exec_shell", args, "no such file")),
            Some(StuckSignal::Warn)
        );
    }

    #[test]
    fn alternating_actions_warn_and_stop_after_two_more_repeats() {
        let a = tool("read_file", json!({"path": "a"}));
        let b = tool("read_file", json!({"path": "b"}));
        let mut guard = StuckGuard::default();
        assert_eq!(guard.observe(a.clone()), None);
        assert_eq!(guard.observe(b.clone()), None);
        assert_eq!(guard.observe(a.clone()), None);
        assert_eq!(guard.observe(b.clone()), Some(StuckSignal::Warn));
        assert_eq!(guard.observe(a.clone()), None);
        assert_eq!(guard.observe(b.clone()), Some(StuckSignal::Stop));
    }

    #[test]
    fn repeated_no_tool_messages_are_detected() {
        let step = StepFingerprint::assistant_no_tool("I need to try again.");
        let mut guard = StuckGuard::default();
        assert_eq!(guard.observe(step.clone()), None);
        assert_eq!(guard.observe(step.clone()), None);
        assert_eq!(guard.observe(step.clone()), None);
        assert_eq!(guard.observe(step.clone()), Some(StuckSignal::Warn));
    }

    #[test]
    fn changed_arguments_reset_the_episode() {
        let mut guard = StuckGuard::default();
        let same = tool("read_file", json!({"path": "a"}));
        let progress = tool("read_file", json!({"path": "b"}));
        assert_eq!(guard.observe(same.clone()), None);
        assert_eq!(guard.observe(same), None);
        assert_eq!(guard.observe(progress.clone()), None);
        assert_eq!(guard.observe(progress), None);
    }

    #[test]
    fn argument_object_key_order_does_not_change_fingerprint() {
        assert_eq!(
            tool("x", json!({"a": 1, "b": 2})),
            tool("x", json!({"b": 2, "a": 1}))
        );
    }
}
