use std::sync::Arc;
use std::ops::Deref;

/// A generic container enum that provides flexible ownership for various data types.
///
/// - `Borrowed`: For data that can be safely borrowed for a lifetime `'a`.
/// - `Owned`: For data that is owned via a `Box<T>`.
/// - `Shared`: For data that is shared via an `Arc<T>`.
#[derive(Debug)]
pub enum Container<'a, T: ?Sized> {
    /// Represents borrowed data with lifetime `'a`.
    Borrowed(&'a T),
    /// Represents owned data, typically heap-allocated via `Box`.
    Owned(Box<T>),
    /// Represents shared data, atomically reference-counted via `Arc`.
    Shared(Arc<T>),
}

// --- Generic From implementations ---

impl<'a, T: ?Sized> From<&'a T> for Container<'a, T> {
    fn from(value: &'a T) -> Self {
        Container::Borrowed(value)
    }
}

impl<'a, T: ?Sized> From<Box<T>> for Container<'a, T> {
    fn from(value: Box<T>) -> Self {
        Container::Owned(value)
    }
}

impl<'a, T: ?Sized> From<Arc<T>> for Container<'a, T> {
    fn from(value: Arc<T>) -> Self {
        Container::Shared(value)
    }
}

// --- AsRef and Deref implementations ---

impl<'a, T: ?Sized> AsRef<T> for Container<'a, T> {
    fn as_ref(&self) -> &T {
        match self {
            Container::Borrowed(value) => value,
            Container::Owned(value) => value.as_ref(),
            Container::Shared(value) => value.as_ref(),
        }
    }
}

impl<'a, T: ?Sized> Deref for Container<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

// --- Generic Clone implementation ---

// Clone for Container<T> where T is Sized and Clone
impl<'a, T: Sized + Clone> Clone for Container<'a, T> {
    fn clone(&self) -> Self {
        match self {
            Container::Borrowed(value) => Container::Borrowed(value), // &'a T where T: Sized, so &'a T is Copy
            Container::Owned(value) => Container::Owned(value.clone()), // value is Box<T>, T is Sized + Clone
            Container::Shared(value) => Container::Shared(Arc::clone(value)),
        }
    }
}
