/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

#[derive(Clone, Debug)]
pub struct Error {
    messages: Vec<String>,
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn new<S>(message: S) -> Self
    where
        S: Into<String>,
    {
        Self {
            messages: vec![message.into()],
        }
    }

    pub fn empty() -> Self {
        Self {
            messages: Default::default(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    pub fn messages(&self) -> &Vec<String> {
        &self.messages
    }

    pub fn push(&mut self, message: String) {
        self.messages.push(message);
    }
}

impl From<reqwest::header::InvalidHeaderValue> for Error {
    fn from(error: reqwest::header::InvalidHeaderValue) -> Self {
        Self {
            messages: vec![format!("{}", error)],
        }
    }
}

impl From<std::string::FromUtf8Error> for Error {
    fn from(error: std::string::FromUtf8Error) -> Self {
        Self {
            messages: vec![format!("{}", error)],
        }
    }
}

impl From<tokio::task::JoinError> for Error {
    fn from(error: tokio::task::JoinError) -> Self {
        Self {
            messages: vec![format!("{}", error)],
        }
    }
}

impl From<dialoguer::Error> for Error {
    fn from(error: dialoguer::Error) -> Self {
        Self {
            messages: vec![format!("{}", error)],
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self {
            messages: vec![format!("{}", error)],
        }
    }
}

impl From<git2::Error> for Error {
    fn from(error: git2::Error) -> Self {
        Self {
            messages: vec![format!("{}", error)],
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self {
            messages: vec![format!("{}", error)],
        }
    }
}

impl From<octocrab::Error> for Error {
    fn from(error: octocrab::Error) -> Self {
        match error {
            octocrab::Error::GitHub { source, backtrace } => {
                let content = if backtrace.status() == std::backtrace::BacktraceStatus::Disabled {
                    format!("GitHub Error: {source}")
                } else {
                    format!("GitHub Error: {source}: {backtrace:?}")
                };
                Self {
                    messages: vec![content],
                }
            }
            e => Self {
                messages: vec![format!("{}", e)],
            },
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = self.messages.last();
        if let Some(message) = message {
            write!(f, "{}", message)
        } else {
            write!(f, "unknown error")
        }
    }
}

pub trait ResultExt {
    type Output;

    fn convert(self) -> Self::Output;
    fn context(self, message: String) -> Self::Output;
    fn reword(self, message: String) -> Self::Output;
}

impl<T, E> ResultExt for std::result::Result<T, E>
where
    E: Into<Error>,
{
    type Output = Result<T>;

    fn convert(self) -> Result<T> {
        match self {
            Ok(v) => Ok(v),
            Err(error) => Err(error.into()),
        }
    }

    fn context(self, message: String) -> Result<T> {
        match self {
            Ok(v) => Ok(v),
            Err(e) => {
                let mut err = e.into();
                err.messages.push(message);
                Err(err)
            }
        }
    }

    fn reword(self, message: String) -> Result<T> {
        match self {
            Ok(v) => Ok(v),
            Err(e) => {
                let mut err = e.into();
                err.messages = vec![message];
                Err(err)
            }
        }
    }
}
