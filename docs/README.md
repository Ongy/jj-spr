# jj-spr Documentation

jj-spr is a command line tool for using a stacked-diff workflow with GitHub, built for [Jujutsu](https://github.com/jj-vcs/jj) version control.

The idea behind jj-spr is that your local branch management should not be dictated by your code-review tool. You should be able to send out code for review in individual commits, not branches. With Jujutsu's anonymous heads and stable change IDs, this workflow becomes even more natural - you don't need branches at all.

If you've used Phabricator's `arc` or the original `spr` tool, you'll find jj-spr very familiar, but enhanced with Jujutsu's powerful features.

## Prerequisites

Before using jj-spr, you should:

- **Know Jujutsu basics**: Understand core concepts like changes, change IDs, and working copy (`@`). If you're new to Jujutsu, read the [Jujutsu Tutorial](https://github.com/jj-vcs/jj/blob/main/docs/tutorial.md) first.
- **Understand the stacked-diff concept** (helpful but not required): Familiarity with review-per-commit workflows (like Phabricator or the original `spr` tool) helps, but you can learn as you go.

If you're coming from Git, the key difference is that Jujutsu uses "changes" with stable IDs instead of commits with hashes. Each change maintains its identity even when you amend or rebase it.

## Table of Contents

### Getting Started
- [Installation](./user/installation.md)
- [Set up spr](./user/setup.md)

### How To
- [Create and Land a Simple PR](./user/simple.md)
- [Stack Multiple PRs](./user/stack.md)
- [Format and Update Commit Messages](./user/commit-message.md)

### Reference Guide
- [Configuration](./reference/configuration.md)
- [Commands](./reference/commands.md)

## Quick Start

Here's a complete example to get you started quickly:

```bash
# 1. Install jj-spr (after installing Rust from https://rustup.rs)
git clone https://github.com/LucioFranco/jj-spr.git
cd jj-spr
cargo install --path spr

# 2. Set up the Jujutsu alias
jj config set --user aliases.spr '["util", "exec", "--", "jj-spr"]'

# 3. Initialize in your repository
cd ~/your-jujutsu-repo
jj spr init  # Follow prompts, you'll need a GitHub token

# 4. Create a change
jj new main
# ... edit your files ...
jj describe -m "Add authentication feature"

# 5. Create a PR (operates on @ and all its ancestors)
jj spr push

# 6. Make updates if needed
# ... edit your files ...
jj spr push  # Updates the PR(s) in the stack

# 7. Land on GitHub UI when approved

# 8. Sync your local stack
jj spr sync
```

**Key concepts:**
- `@` = your working copy (where you edit)
- `jj spr push` creates/updates PRs for the current head and its ancestors
- `jj spr sync` cleans up and rebases the entire stack after landing

See the guides below for detailed explanations.

## Rationale

The reason to use jj-spr is that it perfectly aligns with Jujutsu's philosophy: you work with changes, not branches. Jujutsu's anonymous heads mean you never need to create branches for your work. Combined with stable change IDs that survive rebasing and amending, this creates an ideal environment for stacked diffs.

With Jujutsu + jj-spr:
- No branch management overhead - work directly with changes
- Stable change IDs make it easy to track and update specific changes in a stack
- Automatic rebasing keeps your entire stack up-to-date
- Conflicts are tracked as first-class objects, making complex rebases manageable

You can still create bookmarks (Jujutsu's equivalent of branches) if you want, but they're optional. The tool embraces Jujutsu's model where every change is automatically tracked and can be referenced by its stable ID.

### Why Review Changes?

The principle behind jj-spr is **one change per logical unit of work**. Each change should be able to stand on its own: it should have a coherent thesis and be a complete change in and of itself. It should have a clear summary and description. It should leave the codebase in a consistent state: building and passing tests, etc.

In addition, ideally, it shouldn't be possible to further split a change into multiple changes that each stand on their own. If you _can_ split a change that way, you should (and Jujutsu's `jj split` makes this trivial).

What follows from those principles is the idea that **changes, not branches, should be the unit of code review**. The above description of a change also describes the ideal code review: a single, well-described change that leaves the codebase in a consistent state, and that cannot be subdivided further.

Jujutsu's model makes this natural: every change has a stable ID, can be individually addressed and modified, and maintains its identity through rebases. Why should the code review tool require branches when the VCS doesn't?

Following the one-change-per-review principle maintains the invariant that any change on `main` represents a codebase that has been reviewed _in that state_, and that builds and passes tests, etc. This makes it easy to revert changes, and to bisect.
