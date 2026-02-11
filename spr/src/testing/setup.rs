use std::fs;

fn create_test_git_repo<P: AsRef<std::path::Path>>(path: P) -> git2::Repository {
    fs::create_dir(&path).expect("Failed to create bare repo");
    let repo = git2::Repository::init_bare(&path).expect("Failed to init git repo");

    repo
}

fn clone_repo<P: AsRef<std::path::Path>, Q: AsRef<std::path::Path>>(
    origin: P,
    path: Q,
) -> git2::Repository {
    fs::create_dir(&path).expect("Failed to create bare repo");
    let repo = git2::Repository::clone(
        format!(
            "file://{}",
            origin
                .as_ref()
                .to_str()
                .expect("Failed to convert path to str")
        )
        .as_str(),
        &path,
    )
    .expect("Failed to clone repo");

    // Create initial commit
    let signature =
        git2::Signature::now("Test User", "test@example.com").expect("Failed to create signature");
    let tree_id = {
        let mut index = repo.index().expect("Failed to get index");
        index.write_tree().expect("Failed to write tree")
    };
    let tree = repo.find_tree(tree_id).expect("Failed to find tree");

    let initial_oid = repo
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            "Initial commit",
            &tree,
            &[],
        )
        .expect("Failed to create initial commit");
    drop(tree); // Drop the tree reference before moving repo

    let mut remote = repo.find_remote("origin").expect("Failed to find origin");
    remote
        .push(&[format!("{}:refs/heads/main", initial_oid)], None)
        .expect("Failed to push");
    drop(remote);

    // Initialize a Jujutsu repository
    let output = std::process::Command::new("jj")
        .args(["git", "init", "--colocate"])
        .current_dir(&path)
        .output()
        .expect("Failed to run jj git init");

    if !output.status.success() {
        panic!(
            "Failed to initialize jj repo: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Set up basic jj config
    let _ = std::process::Command::new("jj")
        .args(["config", "set", "--repo", "user.name", "Test User"])
        .current_dir(&path)
        .output();

    let _ = std::process::Command::new("jj")
        .args(["config", "set", "--repo", "user.email", "test@example.com"])
        .current_dir(&path)
        .output();

    repo
}

pub fn repo_with_origin() -> (tempfile::TempDir, crate::jj::Jujutsu, git2::Repository) {
    let temp_dir = tempfile::TempDir::new().expect("Failed to create temp directory");
    let bare_path = temp_dir.path().join("bare");
    let repo_path = temp_dir.path().join("clone");

    let bare = create_test_git_repo(bare_path.clone());
    let repo = clone_repo(bare_path.clone(), repo_path.clone());

    let jj = crate::jj::Jujutsu::new(repo).expect("Failed to create JJ object in cloned repo");

    return (temp_dir, jj, bare);
}
