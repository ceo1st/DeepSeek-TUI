# TUI dogfood integration check — 2026-07-12

Lane: `/Volumes/VIXinSSD/CW/worktrees/codewhale-underwater-tui`
Branch: `codex/underwater-tui-20260711`
HEAD (unchanged): `7e760f8ce2422db9130f771a39e4b2842ef96a8c` (ahead 2 of `origin/main`)
Agent role: integration / conflict-check only. **No commit, push, merge, tag, reset, or clean.**

Related sibling receipts (do not treat as install proof):

- `docs/TUI_DOG_008_STATE_MATRIX_2026-07-12.md`
- `docs/TUI_PARALLEL_REVIEW_2026-07-12.md` (clippy `-D warnings` red; framework dead_code)
- Authoritative queue: `/Volumes/VIXinSSD/CW/TUI_DOGFOOD_ISSUES_2026-07-12.md`

## Snapshot (start of this pass)

| Item | Value |
| --- | --- |
| Dirty tracked | locales (7 packs), fleet profile/roster, localization, main, model_inventory, prompts, route_billing, tools/*, large tui surface |
| Untracked (landed modules) | `model_context/`, `tools/subagent/coord.rs`, `composer_chrome.rs`, `motion/`, `phase_strip.rs`, `settings_picker/`, `shell_key_routing.rs`, `work_surface/interaction.rs`, docs |
| Diffstat (early) | ~38 files, +2275 / −491 |
| Diffstat (end of pass) | ~41+ files, +2873 / −946 and still growing |

## Concurrent writers (live during this pass)

Mtimes advanced while verification ran. **Do not steal these hunks.**

| Path | Observed mtimes (PDT) | Notes |
| --- | --- | --- |
| `fleet/profile.rs` | stable `22:01:50` | 001 identity loader — leave alone |
| `fleet/roster.rs` | stable `21:32:15` | 001 tolerant roster — leave alone |
| `tui/ui.rs` | stable `22:12:41` | Fleet identity sticky toast + other shell hunks |
| `tui/keybindings.rs` | `22:16:06` | 002/003 binding source — idle by end |
| `route_billing.rs` | `22:15` → `22:36` | 010 still writing mid-pass |
| `tui/views/fleet_setup.rs` | → `22:40:10` | 007 still writing mid-pass |
| `core/engine.rs` + `engine/tests.rs` + `ui/tests.rs` | appeared mid-pass | foreign engine tweak |
| `core/mod.rs` + `?? core/runtime_contract/` | appeared ~`22:41` | **live incomplete scaffold** |

**No archive/overwrite performed.** No stylistic or product fixes applied. Integration agent introduced zero code edits.

## Hazard scan

### Clear green (wired + tested while tree was coherent)

| Area | Wiring | Locale | Focused tests |
| --- | --- | --- | --- |
| `model_context` | `main.rs` `mod model_context` | n/a | 7 passed |
| `settings_picker` | `tui/mod.rs` | n/a | 7 passed |
| `motion` | `tui/mod.rs` | n/a | 37 matched / passed (incl. FrameRequester + MotionMode) |
| `phase_strip` | `tui/mod.rs` | n/a | 2 passed |
| `composer_chrome` | `tui/mod.rs` | n/a | 4 passed |
| `work_surface` (+ `interaction`) | `work_surface/mod.rs` | `WorkSurfaceStop*` in all complete packs | 24 passed |
| `shell_key_routing` | `tui/mod.rs` | n/a | 8 passed |
| `tools::subagent::coord` | `subagent/mod.rs` + `registry.rs` register | n/a | 6 passed |
| `fleet::profile` / `fleet::roster` | existing | `FleetProfileIdentityVerifyFailed` in complete packs + `ALL_MESSAGE_IDS` | 28 + 8 passed |
| Localization parity | — | complete packs exact sync | 16 passed |
| `route_billing` | — | — | 11 passed (at time of run) |
| `keybindings::` | — | — | 8 passed |

`cargo fmt --all --check` — **PASS**.

`zh-Hant` deliberately omits the new keys (partial pack) — not a hazard.

No duplicate module names / conflicting MessageIds among the three new keys
(`FleetProfileIdentityVerifyFailed`, `WorkSurfaceStopConfirmControl`,
`WorkSurfaceStoppingControl`).

### Transient red — sibling `runtime_contract` scaffold (recovered)

Mid-pass, `crates/tui/src/core/runtime_contract/mod.rs` declared
`manifest` / `progress` / `resources` / `terminal` / `termination` before all
files existed → `E0583` and blocked `edit_last_turn`. By end of pass the sibling
had landed all seven files under `core/runtime_contract/` and
`cargo check -p codewhale-tui --bin codewhale-tui --locked` returned **exit 0**
(unused-import warnings only). **No stubbing by this agent.** Treat as
in-progress Core work, not dogfood merge damage.

Earlier focused suite (fleet/work_surface/model_context/settings/motion/phase/
composer/coord) completed **before** the transient break.

### Soft / known issues (not integration breakages)

| Signal | Status | Classification |
| --- | --- | --- |
| 16 `dead_code` warnings (MotionPolicy helpers, settings_picker transaction layer, `Step::Thinking`, FrameRequester::reset, …) | present | Clippy `-D warnings` will fail install gate — see `TUI_PARALLEL_REVIEW_2026-07-12.md` P0.1 |
| `fleet_setup` suite SIGTERM mid-run; `review_lists_model_permissions_tools_and_scope` FAILED then **passed in isolation** | flake / kill under load / concurrent rebuild locks | Not a stable product failure; sibling still editing `fleet_setup.rs` |
| `start_on_review_previews_inline` SIGTERM under lock contention | inconclusive | Re-run after 007 agent stops writing |
| `config_ui::build_document_reflects_app_state` | **FAIL** `left: Bypass` / `right: Suggest` | Matches dogfood “pre-existing / shared-state” note; likely approval-default drift from Operate/Fleet work — **isolated assertion**, not a mod-wiring break |
| `edit_last_turn` | could not complete (tree compiling broken by `runtime_contract`) | Re-check after scaffold finishes |

### 001 remaining product gaps (not integration)

Identity verify failure path in `ui.rs` uses `set_sticky_status` +
`MessageId::FleetProfileIdentityVerifyFailed` (good). Duplicate-id conflict
still uses hardcoded EN/zh-Hans via `status_message` — still open on the
dogfood list; leave to Fleet agent.

## Verification log (commands actually run)

Serialized (`--test-threads=1`), `--locked`, `-p codewhale-tui --bin codewhale-tui`:

```
cargo fmt --all --check                                    PASS
fleet::profile                                             PASS (28)
fleet::roster                                              PASS (8)
work_surface                                               PASS (24)
model_context                                              PASS (7)
settings_picker                                            PASS (7)
motion                                                     PASS (37 matched)
phase_strip                                                PASS (2)
composer_chrome                                            PASS (4)
tools::subagent::coord                                     PASS (6)
localization::                                             PASS (16)
shell_key_routing                                          PASS (8)
route_billing                                              PASS (11)
keybindings::                                              PASS (8)
fleet_setup (full filter)                                  FAIL/SIGTERM (see soft)
config_ui                                                  FAIL 1 (Bypass vs Suggest)
edit_last_turn                                             blocked by runtime_contract E0583
```

No full `cargo test -p codewhale-tui --bins` green claim — siblings still dirty
and `runtime_contract` unfinished at end of pass.

## Issue closability (honest)

| ID | Unit-test evidence | Closable now? | Blocker |
| --- | --- | --- | --- |
| 004/005/006 | work_surface + interaction green | **Near-closable after install + real-terminal** | 012 binary proof; Hunter eyeball |
| 002/003 | shell_key_routing + keybindings green | **Near-closable after install + PTY** | Advertised chords in Cursor/Terminal.app still need live proof |
| 008 | phase_strip + composer_chrome green; matrix doc exists | **Not closable** | Visual acceptance + install |
| ModelContext / agents coord | model_context + coord green | Feature land OK; not a dogfood stopship id | Clippy dead_code if gated |
| Settings picker / Motion | green unit tests | Framework land OK | Clippy dead_code; theme may not use transaction API yet |
| 001 | profile/roster green; MessageId for identity verify | **Blocked** | Duplicate-id still hardcoded; live Scout-beside-legacy proof on installed binary; toast for all fail paths |
| 007 | fleet mid-write | **Blocked** | Agent still editing `fleet_setup.rs` |
| 010 | route_billing unit green | **Near / blocked** | Agent still editing billing; install + truthful UI proof |
| 009/011 | not verified this pass | **Blocked** | Need dedicated finish pass |
| 012/013 | — | **Blocked** | Dirty tree, live compile break, clippy `-D warnings` red |

## Recommended next agent

**Finish 001 + 007/010 first — not 012 install.**

1. Confirm `runtime_contract/` has stopped changing (check was green at end of
   this pass; still unused-import noisy and may still be edited).
2. Finish Fleet 001 remainder (duplicate-id MessageId + sticky toast; Scout-beside-legacy live proof) without clobbering profile/roster/ui identity hunks mid-flight.
3. Let 007/010 land or park with a clear WIP note; re-run `fleet_setup` and billing UI slices alone.
4. Then a dedicated **clippy allow / wire / delete** pass for framework dead_code (parallel review P0.1) — required before 012.
5. Only then **TUI-DOG-012** dogfood install + **013** real-terminal matrix.

Installing now would freeze an incomplete concurrent diff and a red clippy gate.

## Hard rails honored

- No push / merge / tag / publish / commit
- No reset / clean / checkout of dirty files
- Fleet dirty hunks and in-progress billing/setup/runtime_contract left untouched
- Receipt only; tree left as found (still dirty, still moving)
