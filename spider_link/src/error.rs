//! Contains all the error handling code for the Spider Link crate.
//!
//! Defines [LinkResult] and [LinkError] to represent error conditions from link
//! operations. These can wrap other types of errors, have the type of the error
//! associated to it, and carry a message describing what went wrong.

use std::{error::Error, fmt::Display};

/// Defines an error returned from the Spider Link crate
pub type LinkResult<T = ()> = Result<T, LinkError>;

/// Trait allows its implementors to be informed of a problem of type P.
#[allow(dead_code)]
pub trait Problem<T, P> {
    /// Set the type of problem for this LinkResult, wrapping if needed
    fn problem(self, kind: P) -> LinkResult<T>;
    /// Set the message for this LinkResult, wrapping if needed
    fn msg<S: Into<String>>(self, msg: S) -> LinkResult<T>;
    /// Set both the type and message for this Link Result, wrapping if needed
    fn problem_msg<S: Into<String>>(self, kind: P, msg: S) -> LinkResult<T>;
}

impl<T, I: Into<LinkError>> Problem<T, ErrorKind> for Result<T, I> {
    fn problem(self, kind: ErrorKind) -> LinkResult<T> {
        match self {
            Ok(value) => Ok(value),
            Err(err) => {
                let err = err.into();
                Err(err.problem(kind))
            }
        }
    }

    fn msg<S: Into<String>>(self, msg: S) -> LinkResult<T> {
        match self {
            Ok(value) => Ok(value),
            Err(err) => {
                let err = err.into();
                Err(err.msg(msg.into()))
            }
        }
    }
    fn problem_msg<S: Into<String>>(self, kind: ErrorKind, msg: S) -> LinkResult<T> {
        match self {
            Ok(value) => Ok(value),
            Err(err) => {
                let err = err.into();
                Err(err.problem(kind).msg(msg))
            }
        }
    }
}

/// Trait to implement for objects that can be wrapped into a LinkResult
pub trait ProblemWrap<T> {
    /// Wrap self, returning a LinkResult
    fn wrap(self) -> LinkResult<T>;
    /// Wrap self, set the problem type, return LinkResult
    fn wrap_problem(self, kind: ErrorKind) -> LinkResult<T>;
    /// Wrap self, set the message, return LinkResult
    fn wrap_msg<S: Into<String>>(self, msg: S) -> LinkResult<T>;
    /// Wrap self, set the problem type and message, return LinkResult
    fn wrap_problem_msg<S: Into<String>>(self, kind: ErrorKind, msg: S) -> LinkResult<T>;
}

impl<T, E: Error + Send + Sync + 'static> ProblemWrap<T> for Result<T, E> {
    fn wrap(self) -> LinkResult<T> {
        self.map_err(|e| LinkError {
            kind: ErrorKind::Misc,
            msg: None,
            source: Some(Box::new(e)),
        })
    }

    fn wrap_problem(self, kind: ErrorKind) -> LinkResult<T> {
        self.wrap().problem(kind)
    }

    fn wrap_msg<S: Into<String>>(self, msg: S) -> LinkResult<T> {
        self.wrap().msg(msg)
    }

    fn wrap_problem_msg<S: Into<String>>(self, kind: ErrorKind, msg: S) -> LinkResult<T> {
        self.wrap().problem(kind).msg(msg)
    }
}

impl<T> ProblemWrap<T> for Option<T> {
    fn wrap(self) -> LinkResult<T> {
        self.ok_or_else(|| LinkError::new())
    }

    fn wrap_problem(self, kind: ErrorKind) -> LinkResult<T> {
        self.ok_or_else(|| LinkError::new().problem(kind))
    }

    fn wrap_msg<S: Into<String>>(self, msg: S) -> LinkResult<T> {
        self.ok_or_else(|| LinkError::new().msg(msg))
    }

    fn wrap_problem_msg<S: Into<String>>(self, kind: ErrorKind, msg: S) -> LinkResult<T> {
        self.ok_or_else(|| LinkError::new().problem(kind).msg(msg))
    }
}

/// Indicate the type of the [LinkError] encountered
#[derive(Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// The link has closed
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

/// The error type for this crate. Used by [LinkResult].
#[derive(Debug)]
pub struct LinkError {
    kind: ErrorKind,
    msg: Option<String>,
    source: Option<Box<(dyn Error + Send + Sync + 'static)>>,
}

impl LinkError {
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
    /// if the LinkError already has a different ErrorKind set,
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

    /// Indicate the message for this LinkError, if the msg is already set, this
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

    pub fn print_report(&self) -> String {
        let mut err: &dyn Error = self;
        let mut ret = String::from("Error:\n");
        loop {
            ret.push_str(&format!("\t{}\n", err));

            if let Some(child) = err.source() {
                err = child;
            } else {
                break;
            }
        }
        ret
    }
}

impl From<ErrorKind> for LinkError {
    fn from(value: ErrorKind) -> Self {
        LinkError {
            kind: value,
            msg: None,
            source: None,
        }
    }
}

impl Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LinkError of kind {:?}", self.kind)?;
        if let Some(msg) = &self.msg {
            write!(f, " with message {msg}")?;
        }
        writeln!(f)
    }
}

impl Error for LinkError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn Error + 'static))
    }
}
