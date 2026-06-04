# v0.9.0 Execution Map

Snapshot date: 2026-06-04

This map tracks the v0.9.0 integration branch and keeps the open-PR harvest
separate from release publishing. It is a working document: update it whenever a
PR is harvested, superseded, deferred, or closed.

## Live Counts

- Actual open issues: 444
- Open PRs: 54
- Repo API open issue count: 498, because GitHub includes PRs in that total
- Open issues labeled `v0.9.0`: 119
- Open issues without a milestone: 100

## Execution Order

1. Stabilization and PR harvest: finish #2721 and #2722 before new feature work.
2. Provider/model/auth correctness: land narrow correctness fixes that match the
   current provider architecture.
3. File decomposition Phase 1: split safe, test-covered config/provider and TUI
   view surfaces before adding larger workflow UX.
4. WhaleFlow MVP: typed IR, executor skeleton, replay, and pod monitor before
   teacher/student promotion loops.
5. Model Lab and HarnessProfile MVP: Hugging Face polish and provider/model
   posture before automatic harness creation.
6. Release readiness: keep #2729 current and do not tag or publish without
   maintainer approval.

## Current Branch Harvest

Branch: `codex/v0.9.0-stewardship`

The branch contains the previous 22-commit v0.9.0 stack plus these fresh
harvest/stewardship commits:

| PR | Disposition | Evidence / next step |
| --- | --- | --- |
| #2708 Windows sub-agent completion halves TUI render width | Cherry-picked as `e933a11d7`; follow-up fix `72653f8ef` invalidates reused fanout-card rows. | `cargo test -p codewhale-tui --locked subagent`; `cargo test -p codewhale-tui --locked terminal_size`; `cargo clippy -p codewhale-tui --locked -- -D warnings` passed. |
| #2627 Xiaomi MiMo Token Plan mode | Harvested only the auth-header behavior as `5aa68d986`; did not merge the conflicting mode/env changes. | `cargo test -p codewhale-tui --bin codewhale-tui --locked xiaomi_mimo`; `cargo test -p codewhale-secrets --locked xiaomi_mimo`; `cargo test -p codewhale-config --locked xiaomi_mimo`; `cargo clippy -p codewhale-tui --locked -- -D warnings` passed. |
| #2636 project-context mtime cache | Defer direct merge; harvest only after cache key/signature is widened. | Must include constitution changes, auto-generated context deletion, canonical path equivalence, and overwrite detection before landing. |
| #2634 HarmonyOS port | Defer direct merge; draft has broad platform and TLS/runtime blast radius. | Harvest at most the unused `rustyline` cleanup after local verification; full port needs OHOS target checks and sandbox/security review. |
| #2687 append-only mode/approval prompt | Defer direct merge; draft has compile failures and Plan-mode prompt correctness risks. | Any future harvest must keep stable `message[0]` genuinely mode-agnostic, preserve mode/approval suffixes after capacity replans, and distinguish external overrides from persisted generated prompts. |
| #2581 provider fallback chain design doc | Manually harvested as `docs/rfcs/2574-provider-fallback-chain.md` because the current PR head has no net file changes. | Keep issue #2574 open for implementation; close/comment on #2581 after the integration branch is public, crediting @idling11 and reporter @hsdbeebou. |

## PR Harvest Queue

| PR | State | v0.9.0 disposition |
| --- | --- | --- |
| #1865 Pro Plan mode | Conflicting | Likely superseded by HarnessProfile/model-posture lane; review before closing. |
| #1893 TLS certificate verification toggle | Conflicting | Security-sensitive; review separately, not part of first v0.9 harvest. |
| #2045 NSIS installer and classroom checklist | Conflicting | Defer unless release-readiness needs Windows installer work. |
| #2048 live shell output | Mergeable | Review against current exec/tool card behavior before merge. |
| #2113 independent scroll regions | Conflicting | Defer; likely overlaps current transcript/sidebar work. |
| #2239 i18n Phase 1-4b | Conflicting | Defer until localization lane. |
| #2242 typed persistent tool permission rules | Conflicting | Compare with #2721 stabilization and permissions model. |
| #2256 workspace crate consolidation | Conflicting | Do not merge during v0.9 stabilization. |
| #2269 approval details and shell previews | Conflicting | Review for small UI harvest only. |
| #2318 message_submit hook transform | Draft/conflicting | Defer; hook behavior must match lifecycle policy. |
| #2382 v0.8.48 release harvest | Draft/conflicting | Candidate to close as obsolete after confirming no unharvested commits. |
| #2476 fork migration parent links | Conflicting | Prior memory says safe candidate; verify against current state before closure/harvest. |
| #2479 ProviderKind/ApiProvider trait collapse | Conflicting | Defer until file decomposition Phase 1 reduces config surface. |
| #2482 WhaleFlow orchestration | Draft/conflicting | Inspect for IR ideas; do not merge wholesale. |
| #2486 WhaleFlow cost tracking | Draft/conflicting | Inspect after #2482; harvest telemetry ideas only. |
| #2491 typed ask permissions schema | Conflicting | Prior memory says safe candidate; verify current permissions work first. |
| #2498 Windows shell process trees | Conflicting | Prior memory says safe candidate; review for #2721 stabilization. |
| #2501 in-process LLM response cache | Conflicting | Defer; cache key risks noted in prior review. |
| #2502 web_run RwLock split | Mergeable | Review lock/panic safety before merge. |
| #2505 subagent cap accounting | Draft/conflicting | Compare with current subagent cap tests before harvest. |
| #2506 provider path suffix overrides | Draft/conflicting | Partly superseded by current provider path-suffix support; verify. |
| #2507 stream chunk timeout config | Draft/conflicting | Defer unless stabilization needs it. |
| #2508 configurable path suffix | Conflicting | Likely superseded by #2506/current code; verify linked issue #2089. |
| #2509 parallel read-only web search | Mergeable | Review for tool-execution scheduler invariants. |
| #2510 custom DuckDuckGo endpoint | Draft/mergeable | Low priority; defer unless docs/search lane takes it. |
| #2511 ToolCallBefore hooks | Conflicting | Defer to hook lifecycle lane. |
| #2512 custom completion sounds | Draft/conflicting | Defer. |
| #2513 restore snapshot listing | Draft/mergeable | Review as small UX polish. |
| #2517 turn_meta tail relocation | Mergeable | Already in high-priority harvest list; review prompt/cache implications. |
| #2520 prompt base disk cache | Mergeable | Review after #2687 prompt architecture decision. |
| #2522 hard compaction preserving system segment | Mergeable | Review after #2687 prompt architecture decision. |
| #2526 shell tool availability docs | Draft/conflicting | Likely superseded by tool-surface docs; verify before closing. |
| #2528 background completion wait | Draft/conflicting | Defer unless failing tests prove need. |
| #2529 workspace shell opt-in | Draft/conflicting | Review with permissions/sandbox stabilization. |
| #2530 mention depth cap hint | Draft/mergeable | Small UX candidate. |
| #2576 PrefixCacheChange events | Mergeable | Review after current prefix-cache commits. |
| #2578 turn_end observer hook | Conflicting | Defer to hook lifecycle lane. |
| #2579 AppendLog session messages | Conflicting | Defer; large architectural change. |
| #2581 provider fallback chain design doc | Mergeable / empty diff | Manually harvested into `docs/rfcs/2574-provider-fallback-chain.md`; close original PR after branch is public, keep #2574 open for implementation. |
| #2623 plan prompt modal scroll support | Mergeable | Already harvested into the 22-commit stack. Comment/close original after integration branch is public. |
| #2627 Xiaomi MiMo Token Plan mode | Conflicting | Partially harvested; leave original open or comment with remaining mode/env scope once branch is public. |
| #2631 estimated_input_tokens cache | Mergeable | Already harvested into the 22-commit stack. |
| #2632 tool-catalog JSON cache | Mergeable | Already harvested into the 22-commit stack. |
| #2633 capacity reverse scans | Mergeable | Already harvested into the 22-commit stack. |
| #2634 HarmonyOS port | Draft/mergeable | Defer broad port. Review found global TLS/provider-install risk, OHOS clipboard/test cfg issues, and major sandbox/process-security degradations. |
| #2635 output rows cache | Mergeable | Already harvested into the 22-commit stack. |
| #2636 project-context cache | Conflicting | Defer/harvest only after cache correctness fixes. |
| #2639 POST /v1/sessions endpoint | Mergeable | Defer; app-server contract needs focused review. |
| #2640 workspace field on UpdateThreadRequest | Mergeable | Defer; app-server contract needs focused review. |
| #2646 release publish hardening | Mergeable | Already harvested into the 22-commit stack. |
| #2687 append-only mode/approval prompt | Draft/mergeable | Defer. Review found compile failures and Agent-mode prompt leakage into Plan sessions via hard-coded prompt refresh. |
| #2708 Windows width fix | Mergeable | Cherry-picked and patched locally. |
| #2730 canonical codewhale settings path | Mergeable | Already harvested into the 22-commit stack. |
| #2732 pausable command lifecycle | Draft/mergeable | Defer; review flagged behavior changes. |

## Issue Reduction Strategy

Issue count should drop through evidence-backed consolidation, not bulk closing.

- Close fixed issues only after the v0.9 integration branch is pushed or merged
  and the relevant tests/checks are named in the closure comment.
- Close obsolete release-harvest PRs/issues after verifying no unique commits or
  linked reports remain.
- Supersede older OPENCODE, memory, web, VS Code, and cache-maximalism tickets
  into the current v0.9 lanes when their acceptance criteria are now covered by
  #2667, #2720-#2729, or a narrower current issue.
- Remove or defer `v0.9.0` scope from valid but non-release-critical roadmap
  issues instead of closing them.
- Always credit PR authors, issue reporters, and useful reviewers when a
  contributor branch is harvested.

## Immediate Next Actions

1. Review #2048, #2502, #2509, #2513, #2530, and #2576 as the next small
   mergeable candidates.
2. Prepare public comments for #2708, #2627, #2634, #2636, #2687, and already-harvested performance
   PRs once this integration branch has a remote review surface.
3. Start file decomposition Phase 1 only after the PR harvest table has no
   unknown high-priority provider/prompt/cache branches.
