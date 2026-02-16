use crate::testing;

#[tokio::test]
async fn fetch_unblocks_push() {
    let (_temp_dir, mut jj, mut gh) = super::push::tests::fore_testing::setup().await;

    super::push::push(
        &mut jj,
        &mut gh,
        &testing::config::basic(),
        super::push::PushOptions::default().with_message(Some("message")),
    )
    .await
    .expect_err("push should fail when upstream is ahead of what we expect");

    super::fetch::fetch(
        super::fetch::FetchOptions::default().with_pull_code(),
        &mut jj,
        &mut gh,
        &testing::config::basic(),
    )
    .await
    .expect("fetch shoudln't fail");

    super::push::push(
        &mut jj,
        &mut gh,
        &testing::config::basic(),
        super::push::PushOptions::default().with_message(Some("message")),
    )
    .await
    .expect("push shouldn't fail after fetch ran in between");
}
