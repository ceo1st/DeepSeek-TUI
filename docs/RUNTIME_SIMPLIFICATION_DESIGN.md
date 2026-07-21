# CodeWhale Runtime Simplification Design

## Goal

Make the model-facing runtime smaller, calmer, and easier for models to use by:

1. Collapsing the long tail of single-purpose file, git, run, and web tools into
   a few canonical action-based tools.
2. Shrinking the system prompt to durable behavioral invariants and per-turn
   permission deltas.
3. Keeping every legacy tool name registered but hidden so old transcripts,
   saved sessions, and recorded automation replay without migration.

## Target model-facing surface (default active)

| Tool | Actions / Niche |
|---|---|
| `Bash` | `run`, `wait`, `interact`, `cancel` (existing) |
| `File` | `read`, `list`, `search_name`, `search_content`, `write`, `edit`, `patch` |
| `Git` | `status`, `diff`, `log`, `show`, `blame` |
| `Run` | `tests`, `verifiers` |
| `Web` | `search`, `fetch`, `wait` (deferred unless network is enabled; hidden aliases for legacy names) |
| `tasks` | durable task family (existing action-based surface) |
| `github` | durable GitHub family (existing; deferred by default) |
| `automation` | durable automation family (existing; deferred by default) |
| `rlm` | durable RLM family (existing; deferred by default) |
| `agent` | sub-agent dispatch |
| `work_update` | progress / plan-of-work updates |
| `update_plan` | plan artifact updates |
| `tool_search` | on-demand discovery of deferred tools |

Default eager count target: **~10** (vs. ~18 today), with the durable families and
`Web` discoverable via `tool_search` when needed.

## Rejected alternatives

- **Keep every tool but defer the rare ones.** This only changes what is
  advertised, not how many distinct schemas the model must learn. It also
  leaves duplicated guidance in the prompt.
- **Route search and git through `Bash`.** `grep_files`, `file_search`, and the
  git tools return structured, workspace-aware output and respect sandbox,
  `.gitignore`, and network policy. Shell would force the model to re-parse
  free-form text and lose those guarantees, so dedicated tools win.
- **One mega `File` tool plus a separate `Edit` tool.** A single `File` tool is
  only slightly larger than a read/edit pair and keeps the boundary the model
  already understands (`read` is cheap, `edit` requires prior read). Splitting
  would re-introduce a two-tool alias for the same underlying operations.
- **Delete legacy tools.** Saved transcripts and replay tests rely on the old
  names. Removing them would require a config migration and break reproducibility.
  Hidden aliases avoid both.

## Compatibility

- Legacy names (`read_file`, `write_file`, `edit_file`, `list_dir`, `file_search`,
  `grep_files`, `apply_patch`, `git_status`, `git_diff`, `git_log`, `git_show`,
  `git_blame`, `run_tests`, `run_verifiers`, `web_search`, `fetch_url`,
  `wait_for_dev_server`) stay registered with `model_visible = false`.
- The engine resolves calls by name, so old transcripts replay without changes.
- `DEFAULT_ACTIVE_NATIVE_TOOLS` is updated to list the new canonical names only;
  hidden legacy tools are ignored by catalog construction.

## Prompt simplification

- Replace the tool-calling recipe sections in `AGENT_MODE` and
  `SUBAGENT_OUTPUT_FORMAT` with short references to the canonical tools.
- Reduce mode deltas to permission statements (Agent = write requires approval,
  Plan = no writes or shell, YOLO = auto-approved, Operate = coordinate from
  ordinary messages).
- Keep the `BASE_PROMPT` behavioral invariants, `LANGUAGE_PROMPT`, and
  `OUTPUT_PROMPT` intact.
- Move detailed templates (`COMPACT_TEMPLATE`, sub-agent brief format, planning
  artifact template) out of the stable prefix and into tool schemas or
  conditional blocks.

## Validation

- Provider-free: `scripts/measure-runtime-contract.py` reports active tool count
  and prompt bytes before and after.
- Behavior-preserving: targeted unit tests for `File`, `Git`, `Run`, and `Web`
  dispatch against legacy inputs.
- Regression: `cargo fmt`, `cargo clippy --workspace --all-targets --locked`,
  `cargo test -p codewhale-tui --bin codewhale-tui --locked`, and
  `cargo test --workspace`.

### v0.9.1 receipt

Measured with `python3 scripts/measure-runtime-contract.py` on 2026-07-21:

| Contract | Before | After |
|---|---:|---:|
| Default active tools | 18 | 9 |
| Active tool bytes | ~25,650 | 20,772 |
| Agent-mode instruction bytes | 4,064 | 663 |
| Full system-prompt bytes | 15,842 | 15,368 |

The final active names are `Bash`, `File`, `Git`, `Run`, `agent`, `tasks`,
`update_plan`, `work_update`, and `tool_search`. `File` advertises only read
actions in Plan mode, and its `patch` action appears only when the existing
apply-patch feature is enabled. Hidden aliases remain executable for transcript
replay but are absent from the model catalog.
