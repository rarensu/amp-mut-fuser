use super::core::Container;
use std::ffi::{OsStr, OsString};
use std::sync::Arc;

// --- Specialized From implementations ---

// Specialization for Vec<U> -> Container<'a, [U]>
impl<'a, U> From<Vec<U>> for Container<'a, [U]> {
    fn from(vec: Vec<U>) -> Self {
        Container::Owned(vec.into_boxed_slice())
    }
}

// Specialization for String -> Container<'a, OsStr>
// This converts String to Box<OsStr> for the Owned variant.
impl<'a> From<String> for Container<'a, OsStr> {
    fn from(s: String) -> Self {
        // Convert String -> OsString -> Box<OsStr>
        Container::Owned(OsString::from(s).into_boxed_os_str())
    }
}

// Specialization for OsString -> Container<'a, OsStr>
impl<'a> From<OsString> for Container<'a, OsStr> {
    fn from(os_string: OsString) -> Self {
        Container::Owned(os_string.into_boxed_os_str())
    }
}

// Allow From<&'a str> to create Container<'a, OsStr> specifically for the Borrowed case
impl<'a> From<&'a str> for Container<'a, OsStr> {
    fn from(s: &'a str) -> Self {
        Container::Borrowed(OsStr::new(s))
    }
}


// --- Specialized Clone implementations ---

// Clone for Container<'a, [U]> where U: Clone
impl<'a, U: Clone> Clone for Container<'a, [U]> {
    fn clone(&self) -> Self {
        match self {
            Container::Borrowed(slice) => Container::Borrowed(slice), // &'a [U] is Copy
            Container::Owned(boxed_slice) => {
                // Box<[U]> is Clone if U: Clone. It creates a new Box<[U]> with cloned content.
                Container::Owned(boxed_slice.clone())
            }
            Container::Shared(arc_slice) => Container::Shared(Arc::clone(arc_slice)),
        }
    }
}

// Clone for Container<'a, str>
impl<'a> Clone for Container<'a, str> {
    fn clone(&self) -> Self {
        match self {
            Container::Borrowed(s_slice) => Container::Borrowed(s_slice), // &'a str is Copy
            Container::Owned(boxed_str) => {
                // Box<str> is Clone. It creates a new Box<str> with cloned content.
                Container::Owned(boxed_str.clone())
            }
            Container::Shared(arc_str) => Container::Shared(Arc::clone(arc_str)),
        }
    }
}

// Clone for Container<'a, OsStr>
impl<'a> Clone for Container<'a, OsStr> {
    fn clone(&self) -> Self {
        match self {
            Container::Borrowed(os_str_slice) => Container::Borrowed(os_str_slice), // &'a OsStr is Copy
            Container::Owned(boxed_os_str) => {
                // Box<OsStr> is Clone. It creates a new Box<OsStr> with cloned content.
                Container::Owned(boxed_os_str.clone())
            }
            Container::Shared(arc_os_str) => Container::Shared(Arc::clone(arc_os_str)),
        }
    }
}
