# Format and Update Commit Messages

In Jujutsu, commit messages (called "descriptions") follow a similar format to traditional Git commits, but with some jj-specific considerations.

## Message Format

You should format your change descriptions like this:

```
One-line title

Then a description, which may be multiple lines long.
This describes the change you are making with this commit.

Reviewers: github-username-a, github-username-b
```

The first line will be the title of the PR created by `jj spr push`, and the rest of the lines except for the `Reviewers` line will be the PR description (i.e. the content of the first comment). The GitHub users named on the `Reviewers` line will be added to the PR as reviewers.

## Working with Jujutsu Descriptions

Set or update a change description:
```shell
# Interactive editor (recommended for multi-line descriptions)
jj describe

# Or set directly from command line
jj describe -m "Add feature

This is a really cool feature!"
```

View the current description:
```shell
jj log --no-graph -r @
```

## Updating the PR Title and Description

When you create a PR with `jj spr push`, the commit message is used to populate the PR title and description.

If you want to update the title or description:

1. **Modify the PR through GitHub's UI** (simplest method). If you do this, you can sync the changes back to your local commit using:
   ```shell
   jj spr fetch
   ```

2. **Update locally and push**:
   ```shell
   # Edit the description
   jj describe
   
   # Push the update to the PR
   jj spr push
   ```

## Fields Added by jj spr

At various stages, `jj spr` will add metadata to your change description:

1. **After creating a PR**, `jj spr push` adds:
   ```
   Pull Request: https://github.com/example/project/pull/123
   ```
   This line tells `jj spr` that a PR exists for this change.

## Example Lifecycle

### Initial description:
```
Add user authentication

Implements JWT-based authentication for the API.

Reviewers: alice, bob
```

### After `jj spr push`:
```
Add user authentication

Implements JWT-based authentication for the API.

Reviewers: alice, bob

Pull Request: https://github.com/example/api/pull/456
```

## Jujutsu-Specific Tips

1. **Change IDs are stable**: Unlike Git commit hashes, Jujutsu change IDs remain the same even when you modify the description.

2. **Bulk operations**: Update multiple descriptions at once:
   ```shell
   # Reword multiple changes interactively
   jj reword -r 'mine() & ~main@origin'
   ```
