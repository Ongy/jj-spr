# Stack Multiple PRs

The differences between jj-spr's commit-based workflow and GitHub's default branch-based workflow are most apparent when you have multiple reviews in flight at the same time.

This guide assumes you're already familiar with the workflow for [simple, non-stacked PRs](./simple.md).

## Dependent Stacks

In Jujutsu, managing stacked changes is natural because Jujutsu maintains stable change IDs and automatically handles rebasing operations.

### Creating Dependent Changes

This is for when the second change literally won't work without the first.

1. Create your first change on top of `main`:
   ```shell
   jj new main@origin
   # Make changes...
   jj describe -m "Add authentication module"
   ```

2. Create your second change on top of the first:
   ```shell
   jj new
   # Make changes that depend on the authentication module...
   jj describe -m "Add user profile endpoints"
   ```

3. Your stack looks like:
   - `@` = "Add user profile endpoints" (second PR change)
   - `@-` = "Add authentication module" (first PR change)

4. Run `jj spr push --all` to create PRs for all changes in your stack:
   ```shell
   jj spr push --all
   ```

   This will create a PR for each change in your stack from `@` back to `main@origin`.

## Understanding Your Stack

Use `jj log` to visualize your stack:
```shell
jj log -r 'main@origin..'
```

**Example output:**
```
@  kmkuslkw you@example.com 2024-01-15 11:30:00
│  Add user profile endpoints
○  rlvkpnrw you@example.com 2024-01-15 11:00:00
│  Add authentication module
◆  main@origin
```

In this example:
- `@` = `kmkuslkw` (second change, depends on first)
- `rlvkpnrw` = first change (base of stack)

### Visual: Local Stack vs GitHub PRs

Here's what the above stack looks like locally vs on GitHub:

```
Local Jujutsu State:                 GitHub State:

@  kmkuslkw                     →    PR #124: "Add user profile endpoints"
│  Add user profile endpoints        base: spr/test/add-authentication-module (PR #123's branch)
│                                    branch: spr/test/add-user-profile-endpoints
○  rlvkpnrw                     →    PR #123: "Add authentication module"
│  Add authentication module         base: main
│                                    branch: spr/test/add-authentication-module
◆  main@origin
```

**Key points:**
- Each change has a unique ID (`rlvkpnrw`, `kmkuslkw`)
- jj-spr creates GitHub branches automatically.
- Stacked PRs: PR #124 is based on PR #123's branch.
- When PR #123 lands, PR #124 automatically updates its base on GitHub (handled by GitHub if you land via the UI).

## Updating Changes in the Stack

Suppose you need to update the first change (authentication module with ID `rlvkpnrw`) in response to review feedback.

**Step 1: Find the change ID**

First, identify which change you want to edit:
```shell
jj log -r 'main@origin..'
```

**Method: Direct editing (jj edit)**

1. Edit the change directly:
   ```shell
   jj edit rlvkpnrw  # Use the actual change ID
   # Make your changes...
   ```

2. The changes are automatically absorbed. Jujutsu will automatically rebase descendant changes.

3. Update the PR for that specific change:
   ```shell
   jj spr push
   ```

4. Return to your top change:
   ```shell
   jj edit kmkuslkw
   ```

## Landing Stacked Changes

> 🚨 **IMPORTANT:** Always land changes in order (parent before child).

### Landing Process (Parent Change)

Using our example stack where `rlvkpnrw` (auth module) is the parent and `kmkuslkw` (user profiles) is the child:

1. **Land the parent change on GitHub** (e.g., merge PR #123).

2. **Run `jj spr sync` to update your stack:**
   ```shell
   jj spr sync
   ```

   `jj spr sync` will:
   - Abandon the merged `rlvkpnrw` commit.
   - Rebase `kmkuslkw` onto the updated `main@origin`.
   - Now `kmkuslkw` is based directly on `main`.

3. **Update the remaining PRs:**
   ```shell
   jj spr push
   ```
   This ensures GitHub knows the new base for the remaining PRs.

## Rebasing the Whole Stack

One of the major advantages of Jujutsu is that rebasing your entire stack onto new upstream changes is trivial:

1. Fetch the latest changes:
   ```shell
   jj git fetch
   ```

2. Rebase your stack:
   ```shell
   jj rebase -s <root-change-id> -d main@origin
   ```

   Where `<root-change-id>` is the first change in your stack.

3. Update all PRs:
   ```shell
   jj spr push --all
   ```

## Working with Revsets

Jujutsu's revset language makes it easy to work with stacks:

```shell
# Show all your changes not yet in main
jj log -r 'mine() & ~main@origin'

# Create PRs for all your ready changes
jj spr push --all -r 'mine() & ~main@origin'
```

The Jujutsu + jj-spr workflow makes stacked PRs feel natural and eliminates much of the complexity found in traditional Git-based stacking workflows.
