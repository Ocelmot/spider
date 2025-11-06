//! Contains all the error handling code for the Spider Spider crate.
//!
//! Defines [SpiderResult] and [SpiderError] to represent error conditions from Spider
//! operations. These can wrap other types of errors, have the type of the error
//! associated to it, and carry a message describing what went wrong.

use std::{error::Error, fmt::Display};

/// Defines an error returned from the Spider Spider crate
pub type SpiderResult<T = ()> = Result<T, SpiderError>;

/// Trait allows its implementors to be informed of a problem of type P.
#[allow(dead_code)]
pub trait Problem<T, P> {
    /// Set the type of problem for this SpiderResult, wrapping if needed
    fn problem(self, kind: P) -> SpiderResult<T>;
    /// Set the message for this SpiderResult, wrapping if needed
    fn msg<S: Into<String>>(self, msg: S) -> SpiderResult<T>;
    /// Set both the type and message for this Spider Result, wrapping if needed
    fn problem_msg<S: Into<String>>(self, kind: P, msg: S) -> SpiderResult<T>;
}

impl<T, I: Into<SpiderError>> Problem<T, ErrorKind> for Result<T, I> {
    fn problem(self, kind: ErrorKind) -> SpiderResult<T> {
        match self {
            Ok(value) => Ok(value),
            Err(err) => {
                let err = err.into();
                Err(err.problem(kind))
            }
        }
    }

    fn msg<S: Into<String>>(self, msg: S) -> SpiderResult<T> {
        match self {
            Ok(value) => Ok(value),
            Err(err) => {
                let err = err.into();
                Err(err.msg(msg.into()))
            }
        }
    }
    fn problem_msg<S: Into<String>>(self, kind: ErrorKind, msg: S) -> SpiderResult<T> {
        match self {
            Ok(value) => Ok(value),
            Err(err) => {
                let err = err.into();
                Err(err.problem(kind).msg(msg))
            }
        }
    }
}

/// Trait to implement for objects that can be wrapped into a SpiderResult
pub trait ProblemWrap<T> {
    /// Wrap self, returning a SpiderResult
    fn wrap(self) -> SpiderResult<T>;
    /// Wrap self, set the problem type, return SpiderResult
    fn wrap_problem(self, kind: ErrorKind) -> SpiderResult<T>;
    /// Wrap self, set the message, return SpiderResult
    fn wrap_msg<S: Into<String>>(self, msg: S) -> SpiderResult<T>;
    /// Wrap self, set the problem type and message, return SpiderResult
    fn wrap_problem_msg<S: Into<String>>(self, kind: ErrorKind, msg: S) -> SpiderResult<T>;
}

impl<T, E: Error + Send + Sync + 'static> ProblemWrap<T> for Result<T, E> {
    fn wrap(self) -> SpiderResult<T> {
        self.map_err(|e| SpiderError {
            kind: ErrorKind::Misc,
            msg: None,
            source: Some(Box::new(e)),
        })
    }

    fn wrap_problem(self, kind: ErrorKind) -> SpiderResult<T> {
        self.wrap().problem(kind)
    }

    fn wrap_msg<S: Into<String>>(self, msg: S) -> SpiderResult<T> {
        self.wrap().msg(msg)
    }

    fn wrap_problem_msg<S: Into<String>>(self, kind: ErrorKind, msg: S) -> SpiderResult<T> {
        self.wrap().problem(kind).msg(msg)
    }
}

impl<T> ProblemWrap<T> for Option<T> {
    fn wrap(self) -> SpiderResult<T> {
        self.ok_or_else(|| SpiderError::new())
    }

    fn wrap_problem(self, kind: ErrorKind) -> SpiderResult<T> {
        self.ok_or_else(|| SpiderError::new().problem(kind))
    }

    fn wrap_msg<S: Into<String>>(self, msg: S) -> SpiderResult<T> {
        self.ok_or_else(|| SpiderError::new().msg(msg))
    }

    fn wrap_problem_msg<S: Into<String>>(self, kind: ErrorKind, msg: S) -> SpiderResult<T> {
        self.ok_or_else(|| SpiderError::new().problem(kind).msg(msg))
    }
}

/// Indicate the type of the [SpiderError] encountered
#[derive(Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// The Spider has closed
    Closed,
    /// There was a problem deserializing some data
    Deserialization,
    /// The receiver has already been taken out of here!
    Taken,
    /// Authentication failure
    Authentication,
    /// Other problems
    Misc,
}

/// The error type for this crate. Used by [SpiderResult].
#[derive(Debug)]
pub struct SpiderError {
    kind: ErrorKind,
    msg: Option<String>,
    source: Option<Box<(dyn Error + Send + Sync + 'static)>>,
}

impl SpiderError {
    /// Create a new, blank error. It is of kind [ErrorKind::Misc], with no
    /// message and no underlying source.
    pub fn new() -> Self {
        Self {
            kind: ErrorKind::Misc,
            msg: None,
            source: None,
        }
    }

    /// Indicate the [ErrorKind] of problem that occurred,
    /// if the SpiderError already has a different ErrorKind set,
    /// this function will wrap that error.
    pub fn problem(mut self, kind: ErrorKind) -> Self {
        if self.kind == kind {
            return self;
        } else if self.kind == ErrorKind::Misc {
            self.kind = kind;
            return self;
        } else {
            return Self {
                kind,
                msg: None,
                source: Some(Box::new(self)),
            };
        }
    }

    /// Indicate the message for this SpiderError, if the msg is already set, this
    /// function will wrap the error.
    pub fn msg<S: Into<String>>(mut self, msg: S) -> Self {
        if self.msg.is_none() {
            self.msg = Some(msg.into());
            self
        } else {
            Self {
                kind: ErrorKind::Misc,
                msg: Some(msg.into()),
                source: Some(Box::new(self)),
            }
        }
    }
}

impl From<ErrorKind> for SpiderError {
    fn from(value: ErrorKind) -> Self {
        SpiderError {
            kind: value,
            msg: None,
            source: None,
        }
    }
}

impl Display for SpiderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SpiderError of kind {:?}", self.kind)?;
        if let Some(msg) = &self.msg {
            write!(f, " with message {msg}")?;
        }
        writeln!(f)
    }
}

impl Error for SpiderError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn Error + 'static))
    }
}
