# Task 2 Brief: Implement Commit 2 (Tri-State `CommentMode` Stacked PR)

## Objective
Implement tri-state `CommentMode` (`Auto`, `On`, `Off`), update `PushConfig` and `PushOptions` to use `CommentMode`, update `push()` execution logic, add unit tests, and push as a stacked PR on top of PR #169.

## Repository Root
`/usr/local/google/home/tpeslalz/ongyjj-spr`

## Context & Base Commit
- Base commit: `kpwwwzov` (`940247d4d37dd4`), which contains PR #169.
- Create a new Jujutsu commit on top of `kpwwwzov` using `jj new kpwwwzov`.

## Detailed Instructions
1. Define `CommentMode` in `spr/src/config/push.rs`:
   ```rust
   #[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default, clap::ValueEnum)]
   #[serde(rename_all = "camelCase")]
   pub enum CommentMode {
       #[default]
       Auto,
       On,
       Off,
   }
   ```
   Provide serde deserialization / `FromStr` handling so `"auto"`, `"on"`, `"always"`, `"true"` -> `On`; `"off"`, `"never"`, `"false"` -> `Off`; `"auto"` -> `Auto`.
2. Update `PushConfig` in `spr/src/config/push.rs`:
   ```rust
   fn default_modify_comment() -> CommentMode {
       CommentMode::Auto
   }

   #[serde(default = "default_modify_comment", alias = "modify_comment")]
   pub modify_comment: CommentMode,
   ```
3. Update `PushOptions` in `spr/src/commands/push.rs`:
   - `#[clap(long, value_enum)] pub modify_comment: Option<CommentMode>,`
   - Builder helper: `pub fn with_modify_comment(mut self, val: CommentMode) -> Self`
4. Update `push()` & `update_tree_overviews(...)` in `spr/src/commands/push.rs`:
   - `let comment_mode = opts.modify_comment.unwrap_or(config.push.modify_comment);`
   - Handle `CommentMode::Off`, `CommentMode::On`, and `CommentMode::Auto` (where `Auto` updates comments when stack > 1 change or overview is required).
5. Add unit tests:
   - Config deserialization tests for `"auto"`, `"on"`, `"off"`, `"always"`, `"never"`.
   - Push tests for `CommentMode::Auto`, `CommentMode::On`, `CommentMode::Off`.
6. Create commit on top of `kpwwwzov` (`jj new kpwwwzov`), set description:
   ```markdown
   ✨ push: support tri-state comment modification mode

   Add tri-state CommentMode (auto/on/off) for spr.push.modifyComment and --modify-comment CLI option.

   #### Why

   Support auto, on, and off modes for GitHub PR stack overview comment modification as requested in review feedback.

   #### The code changes include:

   - Introduce `CommentMode` enum (`Auto`, `On`, `Off`) with clap ValueEnum and Serde deserialization.
   - Update `PushConfig.modify_comment` to default to `CommentMode::Auto`.
   - Update `PushOptions.modify_comment` CLI flag to take `CommentMode`.
   - Update `push()` to evaluate `CommentMode` (`Off` skips, `On` always updates, `Auto` updates when stack overview is needed).
   - Add unit tests for comment mode parsing and push behavior.

   go/traj/5a78c624-2436-40b6-8468-55533420e186
   TESTED=UNITTESTS
   RELNOTES=NONE
   TAG=agy
   CONV=5a78c624-2436-40b6-8468-55533420e186
   ```
7. Run `cargo test` to ensure all tests pass.
8. Push stacked PR to GitHub using `cargo run -- push --draft -m "" -r @-`.

## Report File
Write full report to: `/usr/local/google/home/tpeslalz/ongyjj-spr/.superpowers/sdd/2026-08-04-split-update-only-and-tristate-comment-pr-chain-plan/task-2-report.md`
