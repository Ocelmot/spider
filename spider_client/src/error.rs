//! Contains all the error handling code for the Spider Client crate.
//!
//! Defines [ClientResult] and [ClientError] to represent error conditions from Client
//! operations. These can wrap other types of errors, have the type of the error
//! associated to it, and carry a message describing what went wrong.

use std::{error::Error, fmt::Display};

use spider_link::link_set::LinkSetError;

/// Defines an error returned from the Spider Client crate
pub type ClientResult<T = ()> = Result<T, ClientError>;

/// Trait allows its implementors to be informed of a problem of type P.
#[allow(dead_code)]
pub trait Problem<T, P> {
    /// Set the type of problem for this ClientResult, wrapping if needed
    fn problem(self, kind: P) -> ClientResult<T>;
    /// Set the message for this ClientResult, wrapping if needed
    fn msg<S: Into<String>>(self, msg: S) -> ClientResult<T>;
    /// Set both the type and message for this Client Result, wrapping if needed
    fn problem_msg<S: Into<String>>(self, kind: P, msg: S) -> ClientResult<T>;
}

impl<T, I: Into<ClientError>> Problem<T, ErrorKind> for Result<T, I> {
    fn problem(self, kind: ErrorKind) -> ClientResult<T> {
        match self {
            Ok(value) => Ok(value),
            Err(err) => {
                let err = err.into();
                Err(err.problem(kind))
            }
        }
    }

    fn msg<S: Into<String>>(self, msg: S) -> ClientResult<T> {
        match self {
            Ok(value) => Ok(value),
            Err(err) => {
                let err = err.into();
                Err(err.msg(msg.into()))
            }
        }
    }
    fn problem_msg<S: Into<String>>(self, kind: ErrorKind, msg: S) -> ClientResult<T> {
        match self {
            Ok(value) => Ok(value),
            Err(err) => {
                let err = err.into();
                Err(err.problem(kind).msg(msg))
            }
        }
    }
}

/// Trait to implement for objects that can be wrapped into a ClientResult
#[allow(dead_code)]
pub trait ProblemWrap<T> {
    /// Wrap self, returning a ClientResult
    fn wrap(self) -> ClientResult<T>;
    /// Wrap self, set the problem type, return ClientResult
    fn wrap_problem(self, kind: ErrorKind) -> ClientResult<T>;
    /// Wrap self, set the message, return ClientResult
    fn wrap_msg<S: Into<String>>(self, msg: S) -> ClientResult<T>;
    /// Wrap self, set the problem type and message, return ClientResult
    fn wrap_problem_msg<S: Into<String>>(self, kind: ErrorKind, msg: S) -> ClientResult<T>;
}

impl<T, E: Error + Send + Sync + 'static> ProblemWrap<T> for Result<T, E> {
    fn wrap(self) -> ClientResult<T> {
        self.map_err(|e| ClientError {
            kind: ErrorKind::Misc,
            msg: None,
            source: Some(Box::new(e)),
        })
    }

    fn wrap_problem(self, kind: ErrorKind) -> ClientResult<T> {
        self.wrap().problem(kind)
    }

    fn wrap_msg<S: Into<String>>(self, msg: S) -> ClientResult<T> {
        self.wrap().msg(msg)
    }

    fn wrap_problem_msg<S: Into<String>>(self, kind: ErrorKind, msg: S) -> ClientResult<T> {
        self.wrap().problem(kind).msg(msg)
    }
}

impl<T> ProblemWrap<T> for Option<T> {
    fn wrap(self) -> ClientResult<T> {
        self.ok_or_else(|| ClientError::new())
    }

    fn wrap_problem(self, kind: ErrorKind) -> ClientResult<T> {
        self.ok_or_else(|| ClientError::new().problem(kind))
    }

    fn wrap_msg<S: Into<String>>(self, msg: S) -> ClientResult<T> {
        self.ok_or_else(|| ClientError::new().msg(msg))
    }

    fn wrap_problem_msg<S: Into<String>>(self, kind: ErrorKind, msg: S) -> ClientResult<T> {
        self.ok_or_else(|| ClientError::new().problem(kind).msg(msg))
    }
}

/// Indicate the type of the [ClientError] encountered
#[derive(Debug, PartialEq, Eq)]
pub enum ErrorKind {
    
    /// The connection is closed
    Closed,

    /// Failed to deserialize or parse some data
    Deserialize,

    /// Failed to read or write to the disk
    IO,

    /// A link set has failed
    LinkSetError,

    /// Other problems
    Misc,
}

/// The error type for this crate. Used by [ClientResult].
#[derive(Debug)]
pub struct ClientError {
    kind: ErrorKind,
    msg: Option<String>,
    source: Option<Box<dyn Error + Send + Sync + 'static>>,
}

impl ClientError {
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
    /// if the ClientError already has a different ErrorKind set,
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

    /// Indicate the message for this ClientError, if the msg is already set, this
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
        while let Some(child) = err.source() {
            ret.push_str(&format!("\t{}\n", err));


            err = child;
        }
        ret
    }
}

impl From<ErrorKind> for ClientError {
    fn from(value: ErrorKind) -> Self {
        ClientError {
            kind: value,
            msg: None,
            source: None,
        }
    }
}

impl Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ClientError of kind {:?}", self.kind)?;
        if let Some(msg) = &self.msg {
            write!(f, " with message {msg}")?;
        }
        writeln!(f)
    }
}

impl Error for ClientError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn Error + 'static))
    }
}

impl From<LinkSetError> for ClientError {
    fn from(value: LinkSetError) -> Self {
        Self { kind: ErrorKind::LinkSetError, msg: Some(format!("{}", value)), source: Some(Box::new(value)) }
    }
}
