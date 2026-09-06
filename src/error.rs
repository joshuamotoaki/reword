//! One error type: a message for the user plus an optional hint of what to do.

use std::fmt;

#[derive(Debug)]
pub struct Error {
    pub message: String,
    pub hint: Option<String>,
}

impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Error {
            message: message.into(),
            hint: None,
        }
    }

    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)?;
        if let Some(hint) = &self.hint {
            write!(f, "\n  {hint}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::new(e.to_string())
    }
}

impl From<fsrs::FSRSError> for Error {
    fn from(e: fsrs::FSRSError) -> Self {
        let message = match e {
            fsrs::FSRSError::NotEnoughData => "not enough review history".to_string(),
            fsrs::FSRSError::Interrupted => "interrupted".to_string(),
            fsrs::FSRSError::InvalidParameters => "invalid FSRS parameters".to_string(),
            fsrs::FSRSError::OptimalNotFound => "the optimizer did not converge".to_string(),
            fsrs::FSRSError::InvalidInput => "invalid input to the scheduler".to_string(),
            fsrs::FSRSError::InvalidDeckSize => "invalid deck size".to_string(),
        };
        Error::new(format!("fsrs: {message}"))
    }
}

pub type Result<T> = std::result::Result<T, Error>;

macro_rules! bail {
    ($($arg:tt)*) => {
        return Err($crate::error::Error::new(format!($($arg)*)))
    };
}
pub(crate) use bail;
