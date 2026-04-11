#[derive(Debug, clap::Parser, Default)]
pub struct DetachOptions {
    #[clap(long, short = 'r', group = "revs")]
    revset: Option<String>,
}

#[cfg(test)]
impl DetachOptions {
    pub fn with_revset<S>(mut self, revset: Option<S>) -> Self
    where
        S: Into<String>,
    {
        self.revset = revset.map(|s| s.into());
        self
    }
}

pub async fn detach(
    jj: &mut crate::jj::Jujutsu,
    config: &crate::config::Config,
    opts: DetachOptions,
) -> crate::error::Result<()> {
    let revset = opts
        .revset
        .as_ref()
        .map(|s| crate::jj::RevSet::from_arg(s))
        .unwrap_or(crate::jj::RevSet::current());

    // These are only revisions that have a jj-spr style description AND can be modified.
    let revisions = jj.read_revision_range(
        &revset.and(
            &crate::jj::RevSet::description("glob:\"*Pull Request:*\"")
                .without(&crate::jj::RevSet::immutable()),
        ),
    )?;

    if revisions.is_empty() {
        crate::output::output(
            &config.icons.wave,
            "Nothing to be done. Either the revset was empty or none of the revisions have a PR attached.",
        )?;
        return Ok(());
    }

    for mut revision in revisions.into_iter() {
        revision
            .message
            .remove(&crate::message::MessageSection::LastCommit);
        let pr = revision
            .message
            .remove(&crate::message::MessageSection::PullRequest);

        jj.update_revision_message(&revision)?;
        if let Some(pr) = pr {
            crate::output::output(
                &config.icons.info,
                format!("Detached {} from {}", revision.id, pr),
            )?;
        } else {
            // This should be unreachable by the revision selector further up, but ehh...
            crate::output::output(
                &config.icons.error,
                format!("Detached {} from PR but don't know which one", revision.id),
            )?;
        }
    }

    crate::output::output(&config.icons.wave, "Done")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::testing;

    fn create_jujutsu_commit_in_file(
        jj: &mut crate::jj::Jujutsu,
        message: &str,
        file_content: &str,
        path: &str,
    ) -> crate::jj::ChangeId {
        // Create a file
        let file_path = jj
            .git_repo
            .workdir()
            .expect("Failed to extract workdir from JJ handle")
            .join(path);
        std::fs::write(&file_path, file_content).expect("Failed to write test file");

        jj.commit(message).expect("Failed to commit revision");
        crate::jj::ChangeId::from(
            jj.revset_to_change_id(&crate::jj::RevSet::current().parent())
                .expect("Failed to get changeid of '@-'"),
        )
    }

    fn create_jujutsu_commit(
        jj: &mut crate::jj::Jujutsu,
        message: &str,
        file_content: &str,
    ) -> crate::jj::ChangeId {
        create_jujutsu_commit_in_file(jj, message, file_content, "test.txt")
    }

    #[tokio::test]
    async fn detach_revision() {
        let (_tmp_dir, mut jj, _) = testing::setup::repo_with_origin();

        let change = create_jujutsu_commit(
            &mut jj,
            "My commit\n\nPull Request: https://github.com/Ongy/jj-spr/pull/1\nLast Commit: Some Such",
            "content",
        );

        super::detach(
            &mut jj,
            &testing::config::basic(),
            super::DetachOptions::default()
                .with_revset(Some(crate::jj::RevSet::from(&change).as_ref().to_string())),
        )
        .await
        .expect("Detach shouldn't fail");

        let rev = jj
            .read_revision(change)
            .expect("Shouldn't fail to read revision after detaching it");

        assert!(rev.pull_request_number.is_none());
        assert!(
            !rev.message
                .contains_key(&crate::message::MessageSection::PullRequest)
        );
        assert!(
            !rev.message
                .contains_key(&crate::message::MessageSection::LastCommit)
        );
    }
}
