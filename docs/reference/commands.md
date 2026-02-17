# Commands Reference

This page provides a complete reference for all SPR commands.

## Global Options

All commands support the following global options:

- `-h, --help` - Show help information
- `-V, --version` - Show version information

## Commands

### `jj spr init`

Initialize SPR in the current repository. This command prompts for your GitHub Personal Access Token and configures the repository.

**Usage:**
```bash
jj spr init
```

**What it does:**
- Detects GitHub repository from git remotes
- Prompts for GitHub Personal Access Token
- Stores configuration in jj config

---

### `jj spr push`

Create a new or update an existing Pull Request on GitHub.

**Usage:**
```bash
jj spr push [OPTIONS]
```

**Options:**
- `-r, --revset <REVSET>` - Revision(s) to operate on.
  - If not specified, defaults to `@` (current working copy) or all mutable heads if `--all` is used.
- `-a, --all` - Create/update PRs for all changes from base to heads.
- `-m, --message <MSG>` - Message for PR update commits.
- `-f, --force` - Force push even if upstream has unexpected changes.

**Examples:**
```bash
# Create PR for current working copy (default)
jj spr push

# Create PR for specific change
jj spr push -r <change-id>

# Create PRs for all changes in range
jj spr push -r main..@

# Update PR with specific message
jj spr push -m "Address review comments"
```

---

### `jj spr sync`

Pull state from GitHub and merge into local pull requests. This command is used to clean up local changes for PRs that have been landed/merged on GitHub, and to rebase your work onto the latest upstream main.

**Usage:**
```bash
jj spr sync [OPTIONS]
```

**Options:**
- `-r, --revset <REVSET>` - Revision(s) to operate on.
- `-a, --all` - Sync all changes.

**What it does:**
1. Performs `jj git fetch`.
2. Identifies PRs that have been closed or merged on GitHub.
3. Abandons local commits corresponding to landed/merged PRs.
4. Rebases remaining changes onto the remote main branch.

---

### `jj spr fetch`

Update local commit message and optionally code changes with content from GitHub PR.

**Usage:**
```bash
jj spr fetch [OPTIONS]
```

**Options:**
- `-r, --revset <REVSET>` - Revision(s) to operate on.
- `-a, --all` - Fetch updates for all PRs in the stack.
- `--pull-code-changes` - Also merge in any code changes made on GitHub.

**Use case:** When PR title/description has been updated on GitHub (e.g., via the web UI) and you want to sync those changes back to your local commit description.

---

### `jj spr list`

List open Pull Requests on GitHub and their status.

**Usage:**
```bash
jj spr list
```

**Output includes:**
- PR number and title
- Review status (approved, changes requested, etc.)

---

### `jj spr adopt`

Create a new branch with the contents of an existing Pull Request from GitHub.

**Usage:**
```bash
jj spr adopt [PULL_REQUEST] [OPTIONS]
```

**Arguments:**
- `PULL_REQUEST` - The Pull Request number to adopt.

**Options:**
- `--branch-name <NAME>` - Name of the branch to be created. Defaults to `PR-<number>`.
- `--no-checkout` - Create the new branch but do not check out.

---

## Revision Syntax

SPR supports Jujutsu's revision syntax:

- `@` - Current working copy
- `@-` - Parent of working copy
- `<change-id>` - Specific change by ID (e.g., `qpvuntsm`)
- `main@origin` - Remote tracking branch
- `main..@` - Range from main to current
- `a::c` - Inclusive range from a to c

See [Jujutsu revset documentation](https://martinvonz.github.io/jj/latest/revsets/) for more details.
