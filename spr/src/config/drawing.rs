use serde::{Deserialize, Serialize};

static FORK_CHAR: &str = "┣";
static CONT_CHAR: &str = "┃";
static SPACE_CHAR: &str = " ";

fn default_space() -> String {
    String::from(SPACE_CHAR)
}

fn default_fork() -> String {
    String::from(FORK_CHAR)
}

fn default_cont() -> String {
    String::from(CONT_CHAR)
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Drawing {
    #[serde(default = "default_space")]
    pub space: String,
    #[serde(default = "default_fork")]
    pub fork: String,
    #[serde(default = "default_cont")]
    pub cont: String,
}

impl Default for Drawing {
    fn default() -> Self {
        toml::from_str("").expect("Drawing should be defaultable via serde")
    }
}
