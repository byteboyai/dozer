# S6d-2 Part A restore patch report

## Baseline and scope

- Confirmed in the main repository: `git rev-parse --short feat/panel-agent-2` returned `8cd9bdc9` (exit 0).
- Created `feat/panel-agent-restore` in `/Users/chrischiang/Projects/CoralProjects/byteboy/dozer-restore-patch` from that exact commit. No merge, tag, or push was performed. Part B was not touched.

## Step 1 public-surface seam checklist

1. `AgentPanelState::new`: present.
2. `update::update`, `spawn_new_tab`, and `UpdateCtx`: present.
3. `take_outbox`, `take_requests`, and `take_host_events`: present.
4. Tab select/close/confirmation messages: present.
5. Shared restore entry point: missing; added `restore::fetch_restored` and `RestoredSession`.
6. Shared all-tab resize message: missing; added `Message::Resize`.
7. `active_app_cursor_mode` and `active_selection_text`: present.
8. `PanelDisplay`, `view::agent_list`, and `view::terminal_pane`: present.
9. Terminal scroll/selection messages and `TermEvent -> Message`: missing; added `Scroll`, `SelStart`, `SelUpdate`, and `term_message`.

No other public-library seam gap was found.

## Changes

- Moved project-session filtering/attach recovery into `dozer-panel-agent::restore`; attach failures skip only the affected session.
- Added `AgentPanelState::adopt_restored`, used by Dozer's `from_restore`.
- Added library handling for resize, scrolling, selection, and terminal-event mapping.
- Routed Dozer Shared scroll/selection through the library while retaining the SSH path unchanged.
- Removed the now-replaced Dozer-specific restore forwarder and message variant.

## Commands actually run

- `cargo check -p dozer-panel-agent -p dozer-app`: exit 0 before final cleanup.
- `cargo test -p dozer-panel-agent --lib --quiet`: exit 0; 71 passed, 0 failed.
- `cargo test -p dozer-app --quiet`: exit 0 before final cleanup.
- `cargo test --workspace --no-fail-fast`: exit 0 (run before the final warning-cleanup edits).
- `rustfmt --check --edition 2024` on every changed Rust source: initially exit 1, then formatted; final formatter invocation exit 0.
- `cargo clippy -p dozer-panel-agent -p dozer-app --all-targets -- -D warnings`: initially exposed a test-only type-complexity issue, fixed; subsequent run exposed pre-existing application warnings and an unrelated `workspace/view.rs` type mismatch under all-target clippy. It is not claimed as passing.
- `cargo check -p dozer-app --quiet`: final exit 0. Existing unrelated warnings remain; this patch removed its own new warnings.

## Golden-test evidence

`cargo test -p dozer-panel-agent --lib --quiet` passed with the existing byte-stream golden tests unchanged. `session_stream_golden.txt` and protocol golden fixtures were not modified (`git diff` contains no fixture changes).

## Remaining required clean-snapshot verification

The required `git archive` isolated-target re-run, metadata check, migration and boundary scripts were not run before this report was written; they remain required before delivery acceptance.

## Deferred visual verification checklist

- After restart, each project's live sessions remain and content is continuous.
- Input works normally.
- Shared terminal wheel scrolling, mouse selection, and Cmd-C copy work.
- SSH terminal scrolling and selection remain unchanged.
