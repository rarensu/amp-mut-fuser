use super::core::Container;
use crate::container::utils::{LockError, LockGuard};
use std::ffi::{OsStr, OsString};
#[cfg(unix)]
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::sync::Arc; // Keep Arc in case it's needed by other specializations later
use std::str::Utf8Error;
use std::string::FromUtf8Error;

// --- Specialized Error ---

#[derive(Debug, PartialEq)]
pub enum SpecializedError {
    Utf8(Utf8Error),
    FromUtf8(FromUtf8Error),
    LockRequired,
    LockPoisoned,
}

impl From<Utf8Error> for SpecializedError {
    fn from(err: Utf8Error) -> Self {
        SpecializedError::Utf8(err)
    }
}

impl From<FromUtf8Error> for SpecializedError {
    fn from(err: FromUtf8Error) -> Self {
        SpecializedError::FromUtf8(err)
    }
}

// TODO: Consider implementing std::error::Error for SpecializedError
// impl std::error::Error for SpecializedError { ... }

// --- From implementations for Container<'a, u8> ---

impl<'a> From<String> for Container<'a, u8> {
    fn from(s: String) -> Self {
        Container::Vec(s.into_bytes())
    }
}

impl<'a> From<&'a str> for Container<'a, u8> {
    fn from(s: &'a str) -> Self {
        Container::Ref(s.as_bytes())
    }
}

impl<'a> From<OsString> for Container<'a, u8> {
    fn from(s: OsString) -> Self {
        Container::Vec(s.into_vec())
    }
}

impl<'a> From<&'a OsStr> for Container<'a, u8> {
    fn from(s: &'a OsStr) -> Self {
        Container::Ref(s.as_bytes())
    }
}

// --- Specialized methods for Container<'a, u8> ---

impl<'a> Container<'a, u8> {
    // --- String conversion methods ---

    /// Converts the container's content to a `&str`.
    ///
    /// Panics if the container is a mutating variant and not locked (via `as_ref`).
    /// Returns `Err(Utf8Error)` if the byte slice is not valid UTF-8.
    pub fn to_str(&self) -> Result<&str, Utf8Error> {
        std::str::from_utf8(self.as_ref())
    }

    /// Tries to convert the container's content to a `&str`.
    ///
    /// If the container is a non-locking variant, this attempts direct conversion.
    /// If it's a locking variant, it returns `Err(SpecializedError::LockRequired)`
    /// without attempting to acquire a lock.
    pub fn try_to_str(&self) -> Result<&str, SpecializedError> {
        if self.as_ref_is_safe() {
            std::str::from_utf8(self.as_ref()).map_err(SpecializedError::from)
        } else {
            Err(SpecializedError::LockRequired)
        }
    }

    /// Gets a `&str` view into the container's data, potentially acquiring a lock.
    ///
    /// If a lock is acquired, the `LockGuard` is returned alongside the `&str` to ensure
    /// the borrow remains valid.
    /// Returns `Err(SpecializedError::Utf8)` if conversion fails, or
    /// `Err(SpecializedError::LockPoisoned)` if locking fails.
    pub fn get_str(&'a self) -> Result<(&'a str, Option<LockGuard<'a, u8>>), SpecializedError> {
        if self.as_ref_is_safe() {
            let byte_slice = self.as_ref();
            match std::str::from_utf8(byte_slice) {
                Ok(s) => Ok((s, None)),
                Err(e) => Err(SpecializedError::from(e)),
            }
        } else {
            match self.get_slice() {
                Ok((byte_slice, guard)) => {
                    match std::str::from_utf8(byte_slice) {
                        Ok(s) => Ok((s, guard)),
                        Err(e) => Err(SpecializedError::from(e)),
                    }
                }
                Err(LockError::Poisoned) => Err(SpecializedError::LockPoisoned),
            }
        }
    }

    /// Converts the container's content to an owned `String` (strict UTF-8).
    ///
    /// Acquires a lock if necessary.
    /// Returns `Err(SpecializedError::FromUtf8)` if conversion fails, or
    /// `Err(SpecializedError::LockPoisoned)` if locking fails.
    pub fn get_string(&self) -> Result<String, SpecializedError> {
        let (byte_slice, _guard) = self.get_slice().map_err(|e| match e {
            LockError::Poisoned => SpecializedError::LockPoisoned,
        })?;
        String::from_utf8(byte_slice.to_vec()).map_err(SpecializedError::from)
    }

    /// Converts the container's content to an owned `String` (lossy UTF-8).
    ///
    /// Acquires a lock if necessary.
    /// Returns `Err(SpecializedError::LockPoisoned)` if locking fails.
    /// UTF-8 conversion itself is lossy and will not error.
    pub fn get_string_lossy(&self) -> Result<String, SpecializedError> {
        let (byte_slice, _guard) = self.get_slice().map_err(|e| match e {
            LockError::Poisoned => SpecializedError::LockPoisoned,
        })?;
        Ok(String::from_utf8_lossy(byte_slice).into_owned())
    }

    // --- OsString conversion methods ---

    /// Converts the container's content to an `&OsStr`.
    ///
    /// Panics if the container is a mutating variant and not locked (via `as_ref`).
    pub fn to_os_str(&self) -> &OsStr {
        OsStr::from_bytes(self.as_ref())
    }

    /// Tries to convert the container's content to an `&OsStr`.
    ///
    /// If the container is a non-locking variant, this performs direct conversion.
    /// If it's a locking variant, it returns `Err(SpecializedError::LockRequired)`
    /// without attempting to acquire a lock.
    /// Conversion from `&[u8]` to `&OsStr` itself does not fail.
    pub fn try_to_os_str(&self) -> Result<&OsStr, SpecializedError> {
        if self.as_ref_is_safe() {
            Ok(OsStr::from_bytes(self.as_ref()))
        } else {
            Err(SpecializedError::LockRequired)
        }
    }

    /// Gets an `&OsStr` view into the container's data, potentially acquiring a lock.
    ///
    /// If a lock is acquired, the `LockGuard` is returned alongside the `&OsStr`.
    /// Returns `Err(SpecializedError::LockPoisoned)` if locking fails.
    pub fn get_os_str(&'a self) -> Result<(&'a OsStr, Option<LockGuard<'a, u8>>), SpecializedError> {
        if self.as_ref_is_safe() {
            let byte_slice = self.as_ref();
            Ok((OsStr::from_bytes(byte_slice), None))
        } else {
            match self.get_slice() {
                Ok((byte_slice, guard)) => {
                    Ok((OsStr::from_bytes(byte_slice), guard))
                }
                Err(LockError::Poisoned) => Err(SpecializedError::LockPoisoned),
            }
        }
    }

    /// Converts the container's content to an owned `OsString`.
    ///
    /// Acquires a lock if necessary.
    /// Returns `Err(SpecializedError::LockPoisoned)` if locking fails.
    pub fn get_os_string(&self) -> Result<OsString, SpecializedError> {
        let (byte_slice, _guard) = self.get_slice().map_err(|e| match e {
            LockError::Poisoned => SpecializedError::LockPoisoned,
        })?;
        Ok(OsStr::from_bytes(byte_slice).to_os_string())
    }
}