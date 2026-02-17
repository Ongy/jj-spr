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

Create or update Pull Requests on GitHub for a stack of changes.

**Usage:**
```bash
jj spr push [OPTIONS]
```

**Options:**
- `-r, --revset <REVSET>` - Revision(s) to use as heads. Defaults to `@` (current working copy).
- `-a, --all` - Push all mutable heads.
- `-m, --message <MSG>` - Message for PR update commits.
- `-f, --force` - Force push even if upstream has unexpected changes.

**What it does:**
Operates on the specified revision(s) and **all their mutable ancestors** that have a description. For each change in this stack, SPR will create a new PR or update the existing one.

**Examples:**
```bash
# Push PRs for current working copy and all its ancestors
jj spr push

# Push PRs for a specific stack head
jj spr push -r <change-id>

# Update PRs in the stack with a specific message
jj spr push -m "Address review comments"
```

---

### `jj spr sync`

Pull state from GitHub and merge into local pull requests for a stack of changes.

**Usage:**
```bash
jj spr sync [OPTIONS]
```

**Options:**
- `-r, --revset <REVSET>` - Revision(s) to use as heads. Defaults to `@`.
- `-a, --all` - Sync all mutable heads.

**What it does:**
Operates on the specified revision(s) and **all their ancestors** that have existing PRs.
1. Performs `jj git fetch`.
2. Identifies PRs that have been closed or merged on GitHub.
3. Abandons local commits corresponding to landed/merged PRs.
4. Rebases remaining changes in the stack onto the remote main branch.

---

### `jj spr fetch`

Update local commit messages and optionally code changes from GitHub PRs for a stack of changes.

**Usage:**
```bash
jj spr fetch [OPTIONS]
```

**Options:**
- `-r, --revset <REVSET>` - Revision(s) to use as heads. Defaults to `@`.
- `-a, --all` - Fetch updates for all mutable heads.
- `--pull-code-changes` - Also merge in any code changes made on GitHub.

**What it does:**
Operates on the specified revision(s) and **all their mutable ancestors** that have existing PRs. It synchronizes local descriptions with GitHub PR titles and descriptions.

---

### `jj spr list`

List open Pull Requests on GitHub and their status.

**Usage:**
```bash
jj spr list
```

---

### `jj spr adopt`

Create new local changes by pulling an existing Pull Request chain from GitHub.

**Usage:**
```bash
jj spr adopt [PULL_REQUEST] [OPTIONS]
```

**Arguments:**
- `PULL_REQUEST` - The Pull Request number to adopt.

**Options:**
- `--no-checkout` - Create the new branch but do not check out.

**What it does:**
If the specified PR is part of a stack (i.e., its base is another PR branch), `adopt` will recursively pull and create local changes for the **entire chain** of PRs.

---

## Revision Syntax

SPR supports Jujutsu's revision syntax for specifying heads:

- `@` - Current working copy
- `@-` - Parent of working copy
- `<change-id>` - Specific change by ID (e.g., `qpvuntsm`)
- `main@origin` - Remote tracking branch
- `main..@` - Range from main to current
- `a::c` - Inclusive range from a to c

See [Jujutsu revset documentation](https://martinvonz.github.io/jj/latest/revsets/) for more details.
