// I would like for this to be pub const BASIC, but the config constructor is not const :/
// And since the PR in const stores String, its constructor cannot easily be made const
pub fn basic() -> crate::config::Config {
    crate::config::Config::new(
        "test_owner".into(),
        "test_repo".into(),
        "origin".into(),
        "main".into(),
        "spr/test/".into(),
        false,
    )
}
