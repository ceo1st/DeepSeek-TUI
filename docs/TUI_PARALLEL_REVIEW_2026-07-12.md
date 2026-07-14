# Underwater TUI — Parallel Agent Work Review (2026-07-12)

Reviewer pass over the uncommitted + local diff on branch
`codex/underwater-tui-20260711` against HEAD
`7e760f8ce2422db9130f771a39e4b2842ef96a8c`.

Scope: correctness, regressions, sibling conflicts, and the review criteria in
the request (truthful affordances, single focus ownership, motion semantics,
prompt caps/retain, agent-tool honesty, locale parity, gate risk). Analysis was
read-only except this document. `cargo check` and `cargo clippy` were run to
verify build/gate state.

## Verdict

The individual modules are, on the whole, unusually well-built and
well-tested — the interaction ownership, motion policy, WorldState fragments,
and coordination-tool honesty are all genuinely good. **But the branch does not
currently pass the documented pre-push gate** (`cargo clippy --all-targets
--locked -- -D warnings`), which is a hard blocker for install and will fail
TUI-DOG-012/013. This is the one must-fix.

Build state observed:
- `cargo check -p codewhale-tui --bins --tests` → **passes** (warnings only).
- `cargo clippy -p codewhale-tui --all-targets --locked -- -D warnings` →
  **FAILS**: 37 errors (bin) / 22 errors (bin test).

---

## P0 — must fix before install

### P0.1 Clippy `-D warnings` gate is red (blocks install + TUI-DOG-012/013)

`crates/tui/AGENTS.md` names `cargo clippy --workspace --all-targets --locked
-- -D warnings` as a required gate. The branch fails it. codewhale-tui is a
**binary** crate, so `pub` items with no in-crate consumer trip `dead_code`, and
`-D warnings` promotes every warning to an error. The authors anticipated this
for imports (`#[allow(unused_imports)]` is sprinkled on the re-exports) but
missed it at the method/enum/function level, which strongly suggests the full
`--all-targets` clippy gate was not run before handoff.

Two classes of failure:

**(a) Framework-ahead-of-wiring dead code** — new public API that no production
caller uses yet. Either wire it, delete it, or annotate `#[allow(dead_code)]`
with a "public surface for TUI-DOG-00x sibling" note (matching the existing
`unused_imports` allows):

- `model_context/` — `WorldState::{clear,get,is_empty,render_full,render_diff}`,
  `WorldStateSnapshot::{render_text,render_world_diff}`,
  `WorldStateDiff::{render_incremental_text,is_noop}`,
  `FragmentId::{as_str,role,all}`, `FragmentRole::as_str`,
  `FragmentRender::Cleared` variant, and the `WorldStateDiff` re-export
  (`mod.rs:15,17`).
- `tui/settings_picker/transaction.rs` — the **entire** transactional layer
  (`TransactionLog`, `TransactionEvent`, `TransactionCallbacks`,
  `run_preview/commit/rollback/cancel`, `preview/commit/rollback/cancel/
  item_action/last`) is unused in the bin. The theme migration does **not**
  consume it, so `settings_picker::apply_nav_to_log` is dead too. See P1.5.
- `tui/settings_picker/controller.rs` + `option.rs` —
  `options/tabs/active_tab/query/set_query`, `SettingOption::action`,
  `SettingAvailability::{Disabled variant, disabled_reason}`,
  `SettingItemAction::label`.
- `tui/motion/` — `MotionPolicy::{mode,allows_status_spin,min_frame_interval,
  stream_commit_interval,spinner_presentation,spinner_glyph}`,
  `FrameRequester::{clamp_to_frame_cap,reset,request_count,emit_count,
  is_pending}`, `SpinnerPresentation` enum.
- `tools/subagent/coord.rs` — `AgentsInterruptTool::with_caller` (see P1.2),
  plus `SubAgentManager::{queued_mail_depth,child_was_woken}` and the
  `AgentsListTool`/etc. re-exports in `subagent/mod.rs:58`.
- `tui/shell_key_routing.rs` — `composer_owns_printable`.
- `route_billing.rs` — `should_show_footer_cost` (56), `format_usage_chip`
  (173), `UsageChip::label`.
- `tui/app.rs:1539` — `PrefillCommand` variant never constructed;
  `tui/widgets/mod.rs:58` — `Thinking` variant never constructed;
  `tui/widgets/mod.rs:2923` — `COMPOSER_PANEL_HEIGHT`, and
  `composer_min_input_rows` never used.

**(b) Real clippy style lints** (quick, genuine fixes — not just allows):

- `tui/settings_picker/mod.rs:352,353` — `manual_contains`
  (`iter().any(|e| *e == X)` → `contains(&X)`).
- `tui/views/fleet_setup.rs:1851` — `field_reassign_with_default`.
- `tui/work_surface/interaction.rs:130` — `let…else`→`?`
  (`let Some(action) = primary else { return None };` → `let action = primary?;`).
- `tui/footer_ui.rs:966` — another `let…else`→`?`.
- two `collapsible_if` and two `unused_imports`
  (`theme_picker.rs:18` `WorldStateDiff`; `settings_picker/*` `KeyCode`/
  `KeyModifiers`).

Recommendation: run the gate locally and clear it before any install/dogfood.
Prefer wiring or deletion over blanket `#[allow]`; only annotate the surfaces
that are genuinely staged for a named follow-up.

---

## P1 — follow-ups

### P1.1 `agent` deliberate spawn: declared authority is validated but never enforced (truthful-affordance gap)

`tools/subagent/mod.rs` `parse_spawn_request` (deliberate block, ~6885). When
`deliberate=true` it *requires* `type/profile, workspace_policy,
expected_artifact, write_authority, token_budget` and validates the enums — but
none of `workspace_policy`, `write_authority`, or `expected_artifact` are stored
in `SpawnRequest` or threaded into the spawn. Consequences:

- `write_authority: "read_only"` does **not** restrict the child's toolset — a
  child declared read-only can still get write tools.
- `workspace_policy: "worktree"` does **not** create a worktree; only the
  separate `worktree` field does. A caller can satisfy the gate with
  `workspace_policy:"worktree"` and get a shared-checkout child.
- `expected_artifact` is discarded (not surfaced to the child or projection).

This is exactly the "no invented substrate / truthful affordance" concern: the
schema advertises authority the runtime does not honor. Either enforce these
(map `write_authority`→ToolScope, `workspace_policy:"worktree"`→worktree
request, carry `expected_artifact` into the assignment/projection) or downgrade
the schema wording to "declared intent, not enforced" until wired.

### P1.2 `agents/interrupt` self-guard is inert in production

`coord.rs` fails closed on self only when `caller_agent_id` is set via
`AgentsInterruptTool::with_caller(...)`. `register_coordination_tools`
constructs `AgentsInterruptTool::new(...)` **without** `with_caller`, so at
runtime `caller_agent_id == None` and `interrupt_child`'s self check
(`mod.rs:2855`) never triggers. Root is still fail-closed (literal `"root"` ref
at 2849, plus resolve-not-found for the real root id), and the unit test passes
because it calls `with_caller` directly — so the guard is green in tests but
absent in the shipped registration. Thread the caller identity into
`register_coordination_tools`, or confirm children never receive `agents/*`
(in which case `with_caller` is dead — clippy already flags it in P0).

### P1.3 Streaming catch-up never fires in the wired path

`streaming/mod.rs` adds `note_delta_with_backlog` + `set_allow_catch_up` and
`MOTION_CONTRACT.md`/`mode.rs` document "Full motion may catch up under
backlog." But every production caller uses `note_delta` (queued=1); only tests
call `note_delta_with_backlog`. So catch-up is unreachable, and Reduced vs Full
stream identically. Not a regression (the steady 33ms clock is correct and the
reduced-motion contract holds), but the Full-motion acceleration is aspirational
until real queue depth/oldest-age are fed in. Either wire the backlog metrics
into the drain sites (`ui.rs:2261,2384,6891`) or note the contract as pending.

### P1.4 Prompt WorldState cutover — semantic + fidelity notes

`prompts.rs` now returns `SystemPrompt::Blocks` from the live
session/approval path (previously `Text`). Plumbing is sound — every consumer
(`client::system_to_instructions`, `client/anthropic.rs`, `core/engine.rs`
hashing + gate-block injection, `exec_stream_estimate_system_tokens`,
`context_inspector`) already handles `Blocks`, and it is well tested. Residual
concerns:

- **Mislabel**: configured `instructions=[...]` files are packed into
  `FragmentId::Permissions` (marker `cw:ctx:permissions`). Instructions are not
  permissions; the identity is confusing for anyone reading the marked prompt.
- **Wire separator drift**: the OpenAI/DeepSeek path
  (`client::system_to_instructions`) joins blocks with `"\n\n---\n\n"`,
  injecting `---` rules into the model-visible prompt and diverging from
  `system_prompt_flat_text` (joins with `"\n\n"`), which inspectors/tests use.
  Confirm this is intended and that the constitution prefix stays byte-stable
  for prefix-cache reuse (it should, since the constitution is Blocks[0] and the
  separator is deterministic — but this is a one-time content change vs the old
  Text prompt).
- **Partial delivery**: `AgentTopology` and `SkillsTools` fragments are never
  populated in this path (both passed `None`). No stale facts (they're simply
  absent), but the "typed volatile layer" is only half-wired.

### P1.5 settings_picker "theme migration" doesn't use the transaction layer

The framework's transactional preview/commit/rollback (`transaction.rs`) and
`apply_nav_to_log` are unused in the bin (P0.1a). The `mod.rs` doc claims theme
is "migrated," but theme_picker appears to drive only the controller for
nav/layout, not the transaction log. Verify theme preview→cancel actually
reverts the live theme through whatever path theme_picker uses; if the
transaction layer is the intended rollback mechanism, the migration is
incomplete.

### P1.6 FrameRequester is effectively vestigial where wired

In `ui.rs:3714-3722` the loop calls `request_frame` then `take_due` in the same
tick, and `take_due` clears `next_due`, so `due_in` never schedules a future
wake — the animation cadence is still driven by the pre-existing
`last_status_frame.elapsed()` gate. Harmless, but the coalescing scheduler isn't
actually coalescing across widgets yet.

---

## Minor / defensive

- `work_surface/render.rs` `controls_text`: a row with `stop_action` but no
  `primary_action` renders **no** controls, yet `control_zones` would still emit
  a stop hitbox (phantom clickable with no glyph). Not currently reachable —
  `model.rs` never produces stop-without-primary rows — but the two functions
  should stay in lockstep; add the `(false,true,_)` arm to `controls_text` or an
  assert so a future row type can't create an invisible-but-clickable Stop.
- `work_surface/model.rs`: `agent_progress`-fallback workers always get
  `stop_action = Some(...)` regardless of status, so a progress-only entry that
  is actually settled could still advertise Stop until the snapshot refreshes.
  The cached-worker path correctly gates on `worker_is_active`.
- `fleet/profile.rs`: `load_agent_profiles_from_dir` now dedupes ids
  case-insensitively (`to_ascii_lowercase`) and bails on collision — a stricter
  behavior than before. Intended and consistent with the new authoring gate, but
  it could reject a previously-loadable case-differing pair.

---

## Praise / keep

- **work_surface interaction ownership** (`interaction.rs`, `input.rs`,
  `render.rs`): focus / selection / opened-detail / stop-arm modeled as four
  distinct axes; `claim_focus` clears the transcript-selection owner so only one
  region shows selection; hitboxes recorded at render time from the *same*
  glyphs that are drawn (`controls_text`/`control_zones` share the width branch
  and right-align math); row-local arm→confirm with a 4s window and
  arm-clears-on-selection-move. Truthful: control zones exist iff the action
  exists. Strong, thorough tests. This is the model to copy.
- **motion** (`mode.rs`, `frame_requester.rs`): Reduced = steady display clock,
  static-calm glyph, no catch-up, no decorative frames — semantic stillness, not
  a slow typewriter — and the test asserts exactly that (`reduced` and `full`
  share `stream_commit_interval`). Clean separation of the two settings axes +
  runtime force-reduced overlay.
- **model_context** (`fragment.rs`, `world_state.rs`): capped, marker-stable,
  content-hashed fragments with a real retain-unchanged diff; char-boundary-safe
  truncation with a visible marker; constitution kept out of WorldState as the
  cache-stable prefix. Nicely tested.
- **coord agent tools** (`coord.rs`): `agents/message` is honest
  (`woke:false`, queue depth reported, no wake); `agents/followup` on an
  interrupted_continuable child returns the continuation handle and a note that
  live resume is "not automated yet" instead of pretending — exactly the
  message-without-wake / no-invented-substrate honesty asked for.
- **Fleet profile identity gate** (`profile.rs` + `ui.rs` save path): fails
  closed on malformed TOML / invalid id, tolerant of legacy fields (so a stale
  neighbor can't block authoring), case-insensitive collision detection matching
  the loader. Reads as finished, not mid-edit — contrary to the "Fleet dirty"
  worry.
- **localization**: 3 new keys (`FleetProfileIdentityVerifyFailed`,
  `WorkSurfaceStopConfirmControl`, `WorkSurfaceStoppingControl`) added to the
  enum, `ALL_MESSAGE_IDS`, `en.json`, and all 6 complete packs; `zh-Hant`
  correctly excluded as the declared partial pack. MessageId parity looks
  intact (recommend confirming with the parity tests).
- **phase_strip / composer_chrome**: typed placement (live phases above the
  composer, idle/typing below), quiet live band with no key-chorus, chrome sheds
  padding before content. Well tested.

---

## Files that look mid-edit / unsafe to trust yet

The whole set is uncommitted, and the failing clippy gate means the branch as a
whole is not yet gate-clean. Specifically framework-ahead-of-consumer:

- `tui/settings_picker/transaction.rs` — landed ahead of its consumer; unused in
  the bin. Don't assume theme rollback goes through it (P1.5).
- `model_context/` — builder/diff API largely unused outside `prompts.rs`;
  `AgentTopology`/`SkillsTools` half-wired (P1.4).
- `tui/motion/frame_requester.rs` — public API mostly unused; scheduler
  vestigial where wired (P1.6).
- `route_billing.rs` — `format_usage_chip` / `should_show_footer_cost` unused;
  only `usage_chip` is consumed (by `phase_strip`).
- `tools/subagent/coord.rs` — `with_caller` present but not wired into
  registration (P1.2); deliberate-spawn authority validated but not enforced
  (P1.1).

`fleet/profile.rs` and `fleet/roster.rs` themselves look complete and safe;
`fleet/views/fleet_setup.rs` is part of the failing gate only via a test-side
`field_reassign_with_default` lint (P0.1b), not a logic defect spotted here.

## Suggested pre-install checklist

1. `cargo clippy -p codewhale-tui --all-targets --locked -- -D warnings` → green
   (fix P0.1).
2. `cargo test -p codewhale-tui --bins --locked` and the locale parity tests
   (`shipped_complete_packs_have_raw_key_parity_with_english`,
   `message_id_list_english_pack_stay_in_exact_sync`).
3. Decide P1.1 (enforce or re-word deliberate-spawn authority) and P1.2 (thread
   caller identity) before advertising `agents/interrupt` self-safety or
   `write_authority` as real controls.
