# TUI metamorphosis boundary

The underwater TUI is being replaced as a staged molt, not rewritten inside
the legacy god files.

## New owners

- `crates/tui/src/tui/work_surface/` owns transcript-top Tasks, To-do, active
  workers, stable row IDs, focus, scrolling, hitboxes, and row actions.
- `crates/tui/src/route_billing.rs` owns whether a route presents money,
  subscription/quota usage, or local usage. Model IDs never decide this alone.
- `crates/tui/src/tui/underwater.rs` remains the Ocean shell composition owner.

`ui.rs` and `mouse_ui.rs` are adapters: they forward terminal events and apply
typed actions. They must not regain per-surface state or rendering rules.

## Rollback contract

Classic treatment remains the compatibility shell until Ocean passes the full
size/state/device matrix. Reverting Ocean should require changing the shell
composition call sites, not reconstructing code removed from `sidebar.rs`.

Do not delete Classic or its sidebar until all of these are true:

1. `40x12`, `60x16`, `80x24`, `100x32`, and `140x40` pass keyboard and mouse
   interaction checks.
2. Full/reduced motion and Ombre/Flat/Terminal treatments pass live PTY checks.
3. The hermetic TUI suite passes twice.
4. A release build is installed through `scripts/release/install-dogfood.sh`,
   and its commit/SHA receipt matches the running binary.
5. Hunter accepts the live candidate.

After those gates, remove compatibility modules in a dedicated cleanup change;
do not mix their deletion with feature implementation.
