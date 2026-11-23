//! Minimal internal error type for `b3sum`, replacing `anyhow`.

use core::fmt;
use std::io;

pub(crate) type Result<T> = core::result::Result<T, Error>;

pub(crate) enum Error {
    Io(io::Error),
    Msg(String),
}

#[cfg(target_os = "linux")]
impl Error {
    pub(crate) fn raw_os_error(&self) -> Option<i32> {
        match self {
            Error::Io(e) => e.raw_os_error(),
            Error::Msg(_) => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => fmt::Display::fmt(e, f),
            Error::Msg(s) => f.write_str(s),
        }
    }
}

// Forward `Debug` to `Display` so returning `Err(e)` from `fn main()` prints a
// plain human-readable message rather than an enum variant wrapper.
impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Msg(_) => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<String> for Error {
    fn from(s: String) -> Self {
        Error::Msg(s)
    }
}

impl From<&str> for Error {
    fn from(s: &str) -> Self {
        Error::Msg(s.to_owned())
    }
}

#[macro_export]
macro_rules! format_err {
    ($($arg:tt)*) => {
        $crate::err::Error::Msg(::std::format!($($arg)*))
    };
}

#[macro_export]
macro_rules! bail {
    ($($arg:tt)*) => {
        return ::core::result::Result::Err($crate::format_err!($($arg)*))
    };
}

#[macro_export]
macro_rules! ensure {
    ($cond:expr, $($arg:tt)*) => {
        if !$cond {
            $crate::bail!($($arg)*);
        }
    };
}
