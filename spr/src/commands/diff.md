When we diff a commit we need to:
* Determine commits to push by the one that's specified to `$branch@$origin`.
  `jj log -r "$commit-to-proces ~ $branch@$remote"`
* Process PRs in order from closets to origin to farthest
* PRs can be determined via the tags on the jj commit messages, or are created + attached
* #base_head:
  - If the jj#revision has more than one parent: bail
    `jj log --no-graph -T 'parents.map(|c| c.change_id().short()).join(",")' -r @`
  - If the jj#revision is on top of a commit in `$branch@$origin` use that commit.
  - If it's on top of another jj#revision with a PR, we use the pr_branch/HEAD.
  - otherwise: bail
* #old_head: if a PR exists, get from existing $pr_branch, otherwise take #base_head
* Push a new commit with parents #base_head and $old_head and $jj_tree