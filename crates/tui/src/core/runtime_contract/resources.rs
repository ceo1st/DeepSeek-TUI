use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    File,
    TerminalSession,
    Repository,
    NetworkHost,
    RuntimeState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessMode {
    Read,
    Write,
    Exclusive,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ResourceClaim {
    pub kind: ResourceKind,
    pub key: String,
    pub mode: AccessMode,
}

impl ResourceClaim {
    #[must_use]
    pub fn conflicts_with(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.key == other.key
            && (!matches!(self.mode, AccessMode::Read) || !matches!(other.mode, AccessMode::Read))
    }
}

/// Build deterministic parallel batches. Items with no conflicting resource
/// claims share a batch; conflicting items retain their original order.
#[must_use]
pub fn schedule_non_conflicting<T>(items: Vec<(T, BTreeSet<ResourceClaim>)>) -> Vec<Vec<T>> {
    let mut batches: Vec<(Vec<T>, BTreeSet<ResourceClaim>)> = Vec::new();
    for (item, claims) in items {
        if let Some((batch, batch_claims)) = batches.iter_mut().find(|(_, batch_claims)| {
            !claims.iter().any(|claim| {
                batch_claims
                    .iter()
                    .any(|existing| claim.conflicts_with(existing))
            })
        }) {
            batch.push(item);
            batch_claims.extend(claims);
        } else {
            batches.push((vec![item], claims));
        }
    }
    batches.into_iter().map(|(items, _)| items).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim(key: &str, mode: AccessMode) -> BTreeSet<ResourceClaim> {
        BTreeSet::from([ResourceClaim {
            kind: ResourceKind::File,
            key: key.to_string(),
            mode,
        }])
    }

    #[test]
    fn two_reads_share_a_batch_but_write_is_ordered() {
        let batches = schedule_non_conflicting(vec![
            ("read-a", claim("src/lib.rs", AccessMode::Read)),
            ("read-b", claim("src/lib.rs", AccessMode::Read)),
            ("write", claim("src/lib.rs", AccessMode::Write)),
        ]);
        assert_eq!(batches, vec![vec!["read-a", "read-b"], vec!["write"]]);
    }

    #[test]
    fn unrelated_writes_can_run_together() {
        let batches = schedule_non_conflicting(vec![
            ("a", claim("a.rs", AccessMode::Write)),
            ("b", claim("b.rs", AccessMode::Write)),
        ]);
        assert_eq!(batches, vec![vec!["a", "b"]]);
    }
}
