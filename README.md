# Super Pull Requests (SPR)

**The power tool for Jujutsu + GitHub workflows. Single PRs with amend support. Stacked PRs without the complexity.**

A command-line tool that bridges Jujutsu's change-based workflow with GitHub's pull request model. Amend freely in your local repository while keeping reviewers happy with clean, incremental diffs.

> **⚠️ Important: Write Access Required**
>
> Due to GitHub API limitations, SPR requires **write access** to the repository. You must be a collaborator or have write permissions to use SPR. This is a GitHub platform constraint - the API does not support creating PRs from forks without write access to the target repository.
>
> If you're contributing to a project where you don't have write access, you'll need to use the standard fork + PR workflow through the GitHub web interface.

## Why SPR?

### For Everyone: Amend-Friendly PRs
- **Amend freely**: Use Jujutsu's natural `jj squash` and `jj describe` workflow
- **Review cleanly**: Reviewers see clear incremental diffs, not confusing force-push history
- **Update naturally**: Each update creates a new commit on GitHub, preserving review context
- **Land cleanly**: Everything squashes into one perfect commit on merge

### For Power Users: Effortless Stacking
- **Stack with confidence**: Create dependent or independent PRs with automatic rebase handling
- **Land flexibly**: Effortless cleanup and rebase with `jj spr sync` after landing on GitHub.
- **Rebase trivially**: Jujutsu's stable change IDs survive rebases
- **Review independently**: Each PR shows only its changes, not the cumulative stack

**The Problem SPR Solves:**
Jujutsu encourages amending changes. GitHub's review UI breaks with force pushes. SPR bridges this gap by maintaining an append-only PR branch on GitHub while you amend freely locally.

## Quick Start

### Installation

#### From Source

```bash
git clone https://github.com/LucioFranco/jj-spr.git
cd jj-spr
cargo install --path spr
```

This installs the `jj-spr` binary to your `~/.cargo/bin` directory.

#### Set Up as Jujutsu Subcommand

Configure `jj spr` as a subcommand:

```bash
jj config set --user aliases.spr '["util", "exec", "--", "jj-spr"]'
```

### Initial Setup

1. **Initialize in your repository:**
   ```bash
   cd your-jujutsu-repo
   jj spr init
   ```

2. **Provide your GitHub Personal Access Token** when prompted.

### Basic Workflow

```bash
# 1. Create a change
jj new main@origin
echo "new feature" > feature.txt
jj describe -m "Add new feature"

# 2. Submit for review (operates on @ and all its ancestors)
jj spr push

# 3. Amend based on feedback
echo "updated feature" > feature.txt
jj describe  # Edit description if needed
jj spr push  # Updates PR(s) in the stack

# 4. Land when approved via GitHub UI (e.g. Squash and Merge)

# 5. Sync after landing (cleans up the entire stack)
jj spr sync
```

## Key Concepts

- **`@`** = your working copy (where you make edits)
- **`jj spr push`** treatment: It treates specified revision(s) as heads and operates on them and all their **mutable ancestors** that have descriptions.
- **Change IDs** remain stable through rebases, keeping PRs linked

## Commands

### Core Commands

- **`jj spr push`** - Create or update pull requests for a stack of changes
  - Operates on ancestors: `@` by default, or specified via `-r`
  - Updates create new commits on GitHub (reviewers see clean diffs)

- **`jj spr sync`** - Cleanup and rebase a stack after landing PRs on GitHub
  - Operates on ancestors: `@` by default, or specified via `-r`
  - Abandons local commits for merged/closed PRs
  - Rebases remaining work in the stack onto latest upstream main

- **`jj spr fetch`** - Update local commit messages in a stack from GitHub
  - Use `--pull-code-changes` to also pull code updates from GitHub

- **`jj spr list`** - List open pull requests and their status

- **`jj spr adopt`** - Pull an existing PR (and its chain) from GitHub into your local repo

## Documentation

Full documentation is available at **[luciofranco.github.io/jj-spr](https://luciofranco.github.io/jj-spr/)**

## Credits

Super Pull Requests builds on the foundation of:
- Original [spr](https://github.com/getcord/spr) by the Cord team
- [Jujutsu integration](https://github.com/sunshowers/spr) by sunshowers
- [Jujutsu](https://github.com/martinvonz/jj) by Martin von Zweigbergk and contributors

## License

MIT License - see [LICENSE](./LICENSE) for details.
