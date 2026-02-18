# Stack Multiple PRs

The differences between jj-spr's commit-based workflow and GitHub's default branch-based workflow are most apparent when you have multiple reviews in flight at the same time.

In Jujutsu, managing stacked changes is natural because Jujutsu maintains stable change IDs and automatically handles rebasing operations.

## Dependent Stacks

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

4. Run `jj spr push` to create PRs for the entire stack:
   ```shell
   jj spr push
   ```

   > **Note:** `jj spr push` treats the current revision (`@`) as the head and **automatically processes all its mutable ancestors** that have descriptions. This means it will create or update PRs for both changes in your stack with a single command.

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

In this example, running `jj spr push` will create or update PRs for both `kmkuslkw` and `rlvkpnrw`.

## Updating Changes in the Stack

Suppose you need to update the first change (authentication module with ID `rlvkpnrw`) in response to review feedback.

1. Edit the change directly:
   ```shell
   jj edit rlvkpnrw  # Use the actual change ID
   # Make your changes...
   ```

2. Jujutsu will automatically rebase descendant changes (like `kmkuslkw`).

3. Update the PRs for the stack:
   ```shell
   jj spr push
   ```
   
   Since `jj spr push` operates on ancestors, it will update the PR for `rlvkpnrw` and also ensure `kmkuslkw` is correctly updated on GitHub with its new base.

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

   `jj spr sync` operates on the entire stack of ancestors for your current head. It will:
   - Abandon the merged `rlvkpnrw` commit.
   - Rebase `kmkuslkw` onto the updated `main@origin`.
   - Now `kmkuslkw` is based directly on `main`.

3. **Update the remaining PRs:**
   ```shell
   jj spr push
   ```
   This ensures GitHub knows the new base for the remaining PRs in your stack.

## Rebasing the Whole Stack

One of the major advantages of Jujutsu is that rebasing your entire stack onto new upstream changes is trivial. After rebasing locally, simply run:

```shell
jj spr push
```

This will update all PRs in the stack to reflect their new state and base branches on GitHub.
