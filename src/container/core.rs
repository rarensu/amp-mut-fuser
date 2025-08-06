use std::sync::Arc;
use std::borrow::Cow;
use std::ops::Deref;

#[cfg(not(feature = "no-rc"))]
use std::rc::Rc;
#[cfg(all(feature = "locking",not(feature = "no-rc")))]
use std::cell::{Ref, RefCell};
#[cfg(feature = "locking")]
use std::sync::{Mutex,RwLock};
#[cfg(feature = "locking")]
use std::sync::{MutexGuard, RwLockReadGuard, PoisonError};

#[derive(Debug)]
/// A generic container enum that provides flexible ownership models for unsized data types.
/// If there is a borrow, its lifetime is 'a (most likely 'static).
pub enum Container<T: Clone + 'static> {
    // ----- Simple Variants -----
    /// No data.
    Empty,
    /// An owned, fixed-size, heap-allocated slice.
    Box(Box<[T]>),
    /// An owned, growable, heap-allocated vector.
    Vec(Vec<T>),
    /// A borrowed slice.
    Ref(&'static [T]),
    /// A borrowed slice with copy-on-write.
    Cow(Cow<'static, [T]>),
    #[cfg(not(feature = "no-rc"))]
    /// A reusable, fixed-size slice.
    Rc(Rc<[T]>),
    /// A shared, fixed-size slice.
    Arc(Arc<[T]>),
    // ----- Compount Variants -----
    #[allow(clippy::borrowed_box)]
    /// A borrowed, fixed-size, heap-allocated slice.
    RefBox(&'static Box<[T]>),
    /// A borrowed, immutable, heap-allocated vector. 
    RefVec(&'static Vec<T>),
    /// A borrowed, fixed-size, heap-allocated vector, with copy-on-write. 
    CowBox(Cow<'static, Box<[T]>>),
    /// A borrowed, immutable, heap-allocated vactor, with copy-on-write.
    CowVec(Cow<'static, Vec<T>>),
    #[cfg(not(feature = "no-rc"))]
    /// A reusable, fixed-size, heap-allocated slice.
    RcBox(Rc<Box<[T]>>),
    #[cfg(not(feature = "no-rc"))]
    /// A reusable, immutable, heap-allocated vector.
    RcVec(Rc<Vec<T>>),
    /// A shared, fixed-size, heap-allocated slice.
    ArcBox(Arc<Box<[T]>>),
    /// A shared, immutable, heap-allocated vector.
    ArcVec(Arc<Vec<T>>),
    // ----- Locking Variants -----
    #[cfg(all(feature = "locking", not(feature = "no-rc")))]
    /// A reusable, replaceable, heap-allocated slice.
    RcRefCellBox(Rc<RefCell<Box<[T]>>>),
    #[cfg(all(feature = "locking", not(feature = "no-rc")))]
    /// A reusable, growable, heap-allocated vector.
    RcRefCellVec(Rc<RefCell<Vec<T>>>),
    #[cfg(feature = "locking")]
    /// A shared, fixed-size, replacable, heap-allocated slice.
    ArcMutexBox(Arc<Mutex<Box<[T]>>>),
    #[cfg(feature = "locking")]
    /// A shared, growable, heap-allocated vector.
    ArcMutexVec(Arc<Mutex<Vec<T>>>),
    #[cfg(feature = "locking")]
    /// A shared, fixed-size, replacable, heap-allocated slide with multiple readers.
    ArcRwLockBox(Arc<RwLock<Box<[T]>>>),
    #[cfg(feature = "locking")]
    /// A shared, growable, heap-allocated vector with multiple readers.
    ArcRwLockVec(Arc<RwLock<Vec<T>>>),
}

// ----- SafeBorrow from a Container -----

#[derive(Debug)]
/// A value borrowed from a container with flexible ownership models for unsized data types.
pub enum SafeBorrow<'a, T> {
    // ----- Simple Variants -----
    /// No data.
    Empty,
    /// A borrowed reference to a slice.
    Slice(&'a [T]),
    // ----- Locking Variants -----
    #[cfg(all(feature = "locking", not(feature = "no-rc")))]
    /// A borrowed reference to a reusable, replaceable, heap-allocated slice.
    RcRefCellBox(Ref<'a, Box<[T]>>),
    #[cfg(all(feature = "locking", not(feature = "no-rc")))]
    /// A borrowed reference to a reusable, growable, heap-allocated vector.
    RcRefCellVec(Ref<'a, Vec<T>>),
    #[cfg(feature = "locking")]
    /// A borrowed reference to a shared, fixed-size, replacable, heap-allocated slice.
    ArcMutexBox(MutexGuard<'a, Box<[T]>>),
    #[cfg(feature = "locking")]
    /// A borrowed reference to a shared, growable, heap-allocated vector.
    ArcMutexVec(MutexGuard<'a, Vec<T>>),
    #[cfg(feature = "locking")]
    /// A borrowed reference to a shared, fixed-size, replacable, heap-allocated slide with multiple readers.
    ArcRwLockBox(RwLockReadGuard<'a, Box<[T]>>),
    #[cfg(feature = "locking")]
    /// A borrowed reference to a shared, growable, heap-allocated vector with multiple readers.
    ArcRwLockVec(RwLockReadGuard<'a, Vec<T>>),
}

impl<T: Clone> Deref for SafeBorrow<'_, T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        match self {
            SafeBorrow::Empty => &[],
            SafeBorrow::Slice(value) => value,
            #[cfg(all(feature = "locking", not(feature = "no-rc")))]
            SafeBorrow::RcRefCellBox(value) => value,
            #[cfg(all(feature = "locking", not(feature = "no-rc")))]
            SafeBorrow::RcRefCellVec(value) => value,
            #[cfg(feature = "locking")]
            SafeBorrow::ArcMutexBox(value) => value,
            #[cfg(feature = "locking")]
            SafeBorrow::ArcMutexVec(value) => value,
            #[cfg(feature = "locking")]
            SafeBorrow::ArcRwLockBox(value) => value,
            #[cfg(feature = "locking")]
            SafeBorrow::ArcRwLockVec(value) => value,
        }
    }
}

// --- Container to SafeBorrow ---
// --- Container to Reference ---

#[derive(Debug, PartialEq)]
pub enum SafeBorrowError {
    #[cfg(feature = "locking")]
    Poisoned,
}
#[cfg(feature = "locking")]
impl<T> From<PoisonError<T>> for SafeBorrowError {
    fn from(_: PoisonError<T>) -> Self {
        SafeBorrowError::Poisoned
    }
}

impl<T: Clone> Container<T> {
    /// Try to obtain a borrowed slice-like immutable reference from the container.
    /// Will attempt to safely gain access to a locking variant.
    /// # Errors
    /// Returns an error if the source data is held by locking variant and unavailable.
    pub fn lock(&self) -> Result<SafeBorrow<'_, T>, SafeBorrowError> {
        match self {
            // ----- Simple Variants -----
            Container::Empty => Ok(SafeBorrow::Empty),
            Container::Box(value) => Ok(SafeBorrow::Slice(value.as_ref())),
            Container::Vec(value) => Ok(SafeBorrow::Slice(value.as_ref())),
            Container::Ref(value) => Ok(SafeBorrow::Slice(value)),
            Container::Cow(value) => Ok(SafeBorrow::Slice(value.as_ref())),
            #[cfg(not(feature = "no-rc"))]
            Container::Rc(value) => Ok(SafeBorrow::Slice(value.as_ref())),
            Container::Arc(value) => Ok(SafeBorrow::Slice(value.as_ref())),
            // ----- Compound Variants -----
            Container::RefBox(value) => Ok(SafeBorrow::Slice(value.as_ref())),
            Container::RefVec(value) => Ok(SafeBorrow::Slice(value.as_ref())),
            Container::CowBox(value) => Ok(SafeBorrow::Slice(value.as_ref().as_ref())),
            Container::CowVec(value) => Ok(SafeBorrow::Slice(value.as_ref().as_ref())),
            #[cfg(not(feature = "no-rc"))]
            Container::RcBox(value) => Ok(SafeBorrow::Slice(value.as_ref().as_ref())),
            #[cfg(not(feature = "no-rc"))]
            Container::RcVec(value) => Ok(SafeBorrow::Slice(value.as_ref().as_ref())),
            Container::ArcBox(value) => Ok(SafeBorrow::Slice(value.as_ref().as_ref())),
            Container::ArcVec(value) => Ok(SafeBorrow::Slice(value.as_ref().as_ref())),
            // ----- Locking Variants -----
            #[cfg(all(feature = "locking", not(feature = "no-rc")))]
            Container::RcRefCellBox(value) => Ok(SafeBorrow::RcRefCellBox(value.borrow())),
            #[cfg(all(feature = "locking", not(feature = "no-rc")))]
            Container::RcRefCellVec(value) => Ok(SafeBorrow::RcRefCellVec(value.borrow())),
            #[cfg(feature = "locking")]
            Container::ArcMutexBox(value) => Ok(SafeBorrow::ArcMutexBox(value.lock()?)),
            #[cfg(feature = "locking")]
            Container::ArcMutexVec(value) => Ok(SafeBorrow::ArcMutexVec(value.lock()?)),
            #[cfg(feature = "locking")]
            Container::ArcRwLockBox(value) => Ok(SafeBorrow::ArcRwLockBox(value.read()?)),
            #[cfg(feature = "locking")]
            Container::ArcRwLockVec(value) => Ok(SafeBorrow::ArcRwLockVec(value.read()?)),
        }
    }

    /// Returns a borrowed slice &[] from the container.
    /// Only available if locking variants are disabled.
    /// Hint: use `lock()` to handle locking variants.
    #[cfg(not(feature = "locking"))]
    pub fn as_ref(&self) -> &[T] {
        match self {
            // ----- Simple Variants -----
            Container::Empty => &[], // the 'static zero-length slice of type T
            Container::Box(value) => value.as_ref(),
            Container::Vec(value) => value.as_ref(),
            Container::Ref(value) => value,
            Container::Cow(value) => value.as_ref(),
            #[cfg(not(feature = "no-rc"))]
            Container::Rc(value) => value.as_ref(),
            Container::Arc(value) => value.as_ref(),
            // ----- Compound Variants -----
            Container::RefBox(value) => value.as_ref(),
            Container::RefVec(value) => value.as_ref(),
            Container::CowBox(value) => value.as_ref().as_ref(),
            Container::CowVec(value) => value.as_ref().as_ref(),
            #[cfg(not(feature = "no-rc"))]
            Container::RcBox(value) => value.as_ref().as_ref(),
            #[cfg(not(feature = "no-rc"))]
            Container::RcVec(value) => value.as_ref().as_ref(),
            Container::ArcBox(value) => value.as_ref().as_ref(),
            Container::ArcVec(value) => value.as_ref().as_ref(),
            // ----- Locking Variants -----
            /* not allowed */
        }
    }
}

#[cfg(not(feature = "locking"))]
impl<T: Clone> Deref for Container<T> {
    type Target = [T];
    /// When locking variants are disabled, it is safe to dereference Containers.
    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}