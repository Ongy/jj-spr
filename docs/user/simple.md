# Create and Land a Simple PR

This section details the process of putting a single commit up for review, and landing it (pushing it upstream). It assumes you don't have multiple reviews in flight at the same time. That situation is covered in [another guide](./stack.md), but you should be familiar with this single-review workflow before reading that one.

## Understanding @ and @-

Before diving into the workflow, it's crucial to understand Jujutsu's revision symbols:

- **`@`** = your **working copy** (the current state where you make edits)
- **`@-`** = the **parent** of your working copy (typically your last committed change)

**Why this matters for jj-spr:**
- `jj spr push` operates on **`@` and all its mutable ancestors**.

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

   > **Note:** By default, `push` treats `@` as the head and operates on it and all its mutable ancestors that have descriptions.

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

6. Once your PR is approved, land it using the GitHub UI (e.g., "Squash and merge").

7. **After landing, run `jj spr sync` to clean up and rebase:**
   ```shell
   jj spr sync
   ```

   > ⚠️ **IMPORTANT:** `jj spr sync` operates on your current head (`@`) and all its ancestors. It will:
   > - Fetch the latest changes from GitHub.
   > - Abandon any local commits in the stack that have been merged/closed on GitHub.
   > - Rebase the remaining work in your stack onto the latest `main@origin`.

## Working with Change IDs

In Jujutsu, every change has a stable change ID (like `qpvuntsm`). You can use these IDs to refer to specific changes as heads:

```shell
# Create/update PRs for a specific change and its ancestors
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

2. Resolve any conflicts.

3. Run `jj spr push` to update the PR.

4. Now the PR can be merged on GitHub.
