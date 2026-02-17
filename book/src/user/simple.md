# Create and Land a Simple PR

This section details the process of putting a single commit up for review, and landing it (pushing it upstream). It assumes you don't have multiple reviews in flight at the same time. That situation is covered in [another guide](./stack.md), but you should be familiar with this single-review workflow before reading that one.

## Understanding @ and @-

Before diving into the workflow, it's crucial to understand Jujutsu's revision symbols:

- **`@`** = your **working copy** (the current state where you make edits)
- **`@-`** = the **parent** of your working copy (typically your last committed change)

**After running `jj commit`:**
- Your committed change moves to `@-`
- Your working copy `@` becomes empty (ready for new work)

**Why this matters for jj-spr:**
- `jj spr push` defaults to operating on **`@`** (your current working copy)

**When in doubt:** Run `jj log` to see your revision history and where you are.

```
Example jj log output:
@  qpvuntsm you@example.com 2024-01-15 12:00:00
│  Add authentication feature
◆  main@origin
```

In this example:
- `@` is the "Add authentication feature" commit (what you'd create a PR for)

## Basic Workflow with Jujutsu

1. Fetch the latest changes from upstream:
   ```shell
   jj git fetch
   ```

2. Create a new change for your work:
   ```shell
   jj new main@origin
   ```

3. Make your changes and describe them:
   ```shell
   # Edit your files...
   # ... make your changes ...

   # Describe the change
   jj describe -m "Add user authentication

   This implements basic user authentication using JWT tokens."
   ```

   See [this guide](./commit-message.md) for what to put in your commit message.

4. Run `jj spr push` to create a PR for your change:
   ```shell
   jj spr push
   ```

   > **Note:** By default, `push` operates on `@` (the current working copy).

5. Wait for reviewers to approve. If you need to make changes:

   1. Make your edits in your working copy (`@`). Jujutsu automatically tracks the changes.
   2. Update the description if needed:
      ```shell
      jj describe
      ```
   3. Run `jj spr push` to update the PR.
      ```shell
      jj spr push -m "Address review comments"
      ```

      This will update the PR with the new version of your change. You can pass an update message on the command line using the `--message`/`-m` flag.

6. Once your PR is approved, land it using the GitHub UI (e.g., "Squash and merge").

7. **After landing, run `jj spr sync` to clean up and rebase:**
   ```shell
   jj spr sync
   ```

   > ⚠️ **IMPORTANT:** `jj spr sync` will:
   > - Fetch the latest changes from GitHub.
   > - Abandon your local commit since it has been merged.
   > - Rebases your work onto the latest `main@origin`.

## Working with Change IDs

In Jujutsu, every change has a stable change ID (like `qpvuntsm`). You can use these IDs to refer to specific changes:

```shell
# Create a PR for a specific change
jj spr push -r qpvuntsm
```

## When you update

When you run `jj spr push` to update an existing PR, your update will be added to the PR as a new commit, so that reviewers can see exactly what changed. The new commit's message will be what you entered when prompted or provided via `-m`.

The individual commits that you see in the PR are solely for the benefit of reviewers; they will not be reflected in the commit history when the PR is landed. The commit that eventually lands on upstream `main` will always be a single commit, whose message is the title and description from the PR.

## Updating before landing

Unlike Git, Jujutsu automatically maintains your change's identity even when rebasing. However, it is good practice to run `jj spr push` to update the PR before landing if you've rebased onto new upstream changes, ensuring that the PR content matches what will be landed.

## Conflicts on landing

If there are conflicts with upstream `main`, you should resolve them locally:

1. Fetch and rebase your change onto latest upstream `main`:
   ```shell
   jj git fetch
   jj rebase -d main@origin
   ```

2. Resolve any conflicts:
   ```shell
   # Jujutsu will mark conflicts in the files
   # Edit the files to resolve conflicts
   ```

3. Run `jj spr push` to update the PR:
   ```shell
   jj spr push
   ```

4. Now the PR can be merged on GitHub.

## Quick workflow

```shell
# 1. Create a new change and make your edits
jj new main@origin
# ... make changes ...

# 2. Describe your change
jj describe -m "Add feature"

# 3. Create PR (operates on @)
jj spr push

# 4. Make updates if needed
# ... edit files ...
jj spr push -m "Fix bug"  # Update the PR

# 5. After approval, land on GitHub UI

# 6. Sync and clean up
jj spr sync
```

## Troubleshooting

### "I ran `jj spr push` but nothing happened" or "No changes to push"

**Cause:** Your change might be at a different revision than `@` (the default).

**Solutions:**
1. Check where your changes are:
   ```shell
   jj log
   ```

2. If your change is at `@-`:
   ```shell
   jj spr push -r @-
   ```

### "I forgot to sync after landing"

**Problem:** After landing on GitHub, your local commit still exists and is not based on the new `main`.

**Solution:**
```shell
jj spr sync
```

### "The PR content doesn't match what will be landed"

**Cause:** You've made local changes (like rebasing) without updating the PR.

**Solution:**
```shell
jj spr push  # Update the PR to match local state
```
