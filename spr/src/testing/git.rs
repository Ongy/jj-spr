use std::fmt::{Debug, Display};

use git2::{IndexTime, Oid, Time};

pub fn add_commit_and_push_to_remote<B: Display>(repo: &git2::Repository, branch: B) -> Oid {
    let trunk = repo
        .find_commit(
            repo.revparse_single("HEAD")
                .expect("Failed to parse revparse HEAD")
                .id(),
        )
        .expect("Failed to find commit for HEAD");

    add_commit_on_and_push_to_remote(repo, branch, [trunk.id()])
}

pub fn add_commit_on_and_push_to_remote<B: Display, I: IntoIterator<Item = Oid> + Debug>(
    repo: &git2::Repository,
    branch: B,
    parents: I,
) -> Oid {
    add_commit_on_and_push_to_remote_file(repo, branch, parents, "test.txt")
}

pub fn add_commit_on_and_push_to_remote_file<B: Display, I: IntoIterator<Item = Oid> + Debug>(
    repo: &git2::Repository,
    branch: B,
    parents: I,
    path: &str,
) -> Oid {
    let mut index = repo.index().expect("Couldn't get index from git repo");
    let entry = git2::IndexEntry {
        ctime: IndexTime::new(0, 0),
        mtime: IndexTime::new(0, 0),
        dev: 0,
        ino: 0,
        mode: git2::FileMode::Blob.into(),
        uid: 0,
        gid: 0,
        file_size: 0,
        id: Oid::zero(),
        flags: 0,
        flags_extended: 0,
        path: Vec::from(path.as_bytes()),
    };

    let parent_commits: Vec<_> = parents
        .into_iter()
        .map(|oid| {
            repo.find_commit(oid)
                .expect("Failed to find commit for parent")
        })
        .collect();

    let parent_tree = repo.find_tree(
        parent_commits
            .get(0)
            .expect("Should get at least one parent for this function")
            .tree_id(),
    ).expect("Should be able to find tree for parent's tree id");
    index
        .read_tree(&parent_tree)
        .expect("Should be able to read parent tree");

    index
        .add_frombuffer(
            &entry,
            format!("change on {:?} on {}", parent_commits, branch).as_ref(),
        )
        .expect("Expected to be able to read from buffer");

    // This is stupid, but I don't know rust...
    let commit_refs: Vec<_> = parent_commits.iter().collect();

    let sig = git2::Signature::new("User", "user@example.com", &Time::new(0, 0))
        .expect("Failed to build commit signature");
    let tree_oid = index.write_tree().expect("Failed to write tree to disk");
    let tree = repo
        .find_tree(tree_oid)
        .expect("Failed to find tree from OID");
    let commit_oid = repo
        .commit(
            None,
            &sig,
            &sig,
            "Test commit",
            &tree,
            commit_refs.as_slice(),
        )
        .expect("Failed to commit to repo");

    let mut remote = repo
        .find_remote("origin")
        .expect("Expected to find origin as remote");

    remote
        .push(&[format!("{commit_oid}:refs/heads/{branch}")], None)
        .expect("Failed to push");

    commit_oid
}
