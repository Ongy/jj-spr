use std::fmt::Display;

use git2::{IndexTime, Oid, Time};

pub fn add_commit_and_push_to_remote<B: Display>(repo: &git2::Repository, branch: B) -> Oid {
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
        path: Vec::from("test.txt".as_bytes()),
    };
    let trunk = repo
        .find_commit(
            repo.revparse_single("HEAD")
                .expect("Failed to parse revparse HEAD")
                .id(),
        )
        .expect("Failed to find commit for HEAD");
    index
        .add_frombuffer(&entry, format!("change on {} on {}", trunk.id(), branch).as_ref())
        .expect("Expected to be able to read from buffer");
    let sig = git2::Signature::new("User", "user@example.com", &Time::new(0, 0))
        .expect("Failed to build commit signature");
    let tree_oid = index.write_tree().expect("Failed to write tree to disk");
    let tree = repo
        .find_tree(tree_oid)
        .expect("Failed to find tree from OID");
    let commit_oid = repo
        .commit(None, &sig, &sig, "Test commit", &tree, &[&trunk])
        .expect("Failed to commit to repo");

    let mut remote = repo
        .find_remote("origin")
        .expect("Expected to find origin as remote");

    remote
        .push(
            &[format!("{commit_oid}:refs/heads/{branch}")],
            None,
        )
        .expect("Failed to push");

    commit_oid
}
