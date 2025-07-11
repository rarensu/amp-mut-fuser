use super::core::Container;
use std::ops::Deref;
use std::sync::{Arc,Mutex,RwLock};
use std::borrow::Cow;
use std::cell::RefCell;

// --- AsRef and Deref implementations ---

impl<'a, T: Clone> AsRef<[T]> for Container<'a, T> {
    fn as_ref(&self) -> &[T] {
        match self {
            // ----- Simple Variants -----
            Container::Empty => &[], // the 'static zero-length slice of type T
            Container::Box(value) => value.as_ref(),
            Container::Vec(value) => value.as_ref(),
            Container::Ref(value) => value,
            Container::Cow(value) => value.as_ref(),
            Container::Rc(value) => value.as_ref(),
            Container::Arc(value) => value.as_ref(),
            // ----- Compound Variants -----
            Container::RefBox(value) => value /*?*/,
            Container::RefVec(value) => value /*?*/,
            Container::CowBox(value) => value /*?*/,
            Container::CowVec(value) => value /*?*/,
            Container::RcBox(value) => value /*?*/,
            Container::RcVec(value) => value /*?*/,
            Container::ArcBox(value) => value /*?*/,
            Container::ArcVec(value) => value /*?*/,
            // ----- Mutating Variants -----
            Container::RcRefCellBox(value) => value /*?*/,
            Container::RcRefCellVec(value) => value /*?*/,
            Container::ArcMutexBox(value) => value /*?*/,
            Container::ArcMutexVec(value) => value /*?*/,
            Container::ArcRwLockBox(value) => value /*?*/,
            Container::ArcRwLockVec(value) => value /*?*/,
        }
    }
}

impl<'a, T: Clone> Deref for Container<'a, T> {
    // all variants dereference to a slice
    type Target = [T]; 
    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

// --- Generic Clone implementation ---

// Clone for Container<T> where T is Clone
impl<'a, T: Clone> Clone for Container<'a, T> {
    fn clone(&self) -> Self {
        match self {
            // ----- Simple Variants -----
            Container::Empty => &[], // the 'static zero-length slice of type T
            Container::Box(value) => Container::Box(value /*?*/ ),
            Container::Vec(value) => Container::Vec(value /*?*/ ),
            Container::Ref(value) => Container::Ref(value),
            Container::Cow(value) => Container::Cow(value /*?*/ ),
            Container::Rc(value) => Container::Rc(value /*?*/ ),
            Container::Arc(value) => Container::Arc(value /*?*/ ),
            // ----- Compound Variants -----
            Container::RefBox(value) => Container::RefBox(value),
            Container::RefVec(value) => Container::RefVec(value),
            Container::CowBox(value) => Container::CowBox(value /*?*/ ),
            Container::CowVec(value) => Container::CowVec(value /*?*/ ),
            Container::RcBox(value) => Container::RcBox(value /*?*/ ),
            Container::RcVec(value) => Container::RcVec(value /*?*/ ),
            Container::ArcBox(value) => Container::ArcBox(value /*?*/ ),
            Container::ArcVec(value) => Container::ArcVec(value /*?*/ ),
            // ----- Mutating Variants -----
            Container::RcRefCellBox(value) => Container::RcRefCellBox(value /*?*/ ),
            Container::RcRefCellVec(value) => Container::RcRefCellVec(value /*?*/ ),
            Container::ArcMutexBox(value) => Container::ArcMutexBox(value /*?*/ ),
            Container::ArcMutexVec(value) => Container::ArcMutexVec(value /*?*/ ),
            Container::ArcRwLockBox(value) => Container::ArcRwLockBox(value /*?*/ ),
            Container::ArcRwLockVec(value) => Container::ArcRwLockVec(value /*?*/ ),
        }
    }
}

// --- Lock guard enum for uniform API ---

use std::sync::{MutexGuard, RwLockReadGuard, PoisonError};

#[derive(Debug)]
pub enum LockError {
    Poisoned,
}

impl<T> From<PoisonError<T>> for LockError {
    fn from(_: PoisonError<T>) -> Self {
        LockError::Poisoned
    }
}

pub enum LockGuard<'a, T> {
    RefCellBox(std::cell::Ref<'a, Box<[T]>>),
    RefCellVec(std::cell::Ref<'a, Vec<T>>),
    MutexBox(MutexGuard<'a, Box<[T]>>),
    MutexVec(MutexGuard<'a, Vec<T>>),
    RwLockBox(RwLockReadGuard<'a, Box<[T]>>),
    RwLockVec(RwLockReadGuard<'a, Vec<T>>),
}

// --- Easy slice access implementation ---

impl<'a, T: Clone> Container<'a, T> {
    /// Gets a slice reference along with an optional lock guard.
    /// The caller must hold onto the lock guard for the duration of slice usage.
    pub fn get_slice(&'a self) -> Result<(&'a [T], Option<LockGuard<'a, T>>), LockError> {
        match self {
            // ----- Simple Variants -----
            Container::Empty => Ok((&[], None)),
            Container::Box(value) => Ok((value.as_ref(), None)),
            Container::Vec(value) => Ok((value.as_ref(), None)),
            Container::Ref(value) => Ok((value, None)),
            Container::Cow(value) => Ok((value.as_ref(), None)),
            Container::Rc(value) => Ok((value.as_ref(), None)),
            Container::Arc(value) => Ok((value.as_ref(), None)),
            // ----- Compound Variants -----
            Container::RefBox(value) => Ok((value.as_ref(), None)),
            Container::RefVec(value) => Ok((value.as_ref(), None)),
            Container::CowBox(value) => Ok((value.as_ref().as_ref(), None)),
            Container::CowVec(value) => Ok((value.as_ref().as_ref(), None)),
            Container::RcBox(value) => Ok((value.as_ref().as_ref(), None)),
            Container::RcVec(value) => Ok((value.as_ref(), None)),
            Container::ArcVec(value) => Ok((value.as_ref(), None)),
            Container::ArcBox(value) => Ok((value.as_ref().as_ref(), None)),
            // ----- Mutating Variants -----
            Container::RcRefCellBox(value) => {
                let guard = value.borrow();
                let slice_ref = unsafe {
                    // SAFETY: The slice reference is valid as long as the guard is held
                    std::slice::from_raw_parts(guard.as_ptr(), guard.len())
                };
                Ok((slice_ref, Some(LockGuard::RefCellBox(guard))))
            },
            Container::RcRefCellVec(value) => {
                let guard = value.borrow();
                let slice_ref = unsafe {
                    // SAFETY: The slice reference is valid as long as the guard is held
                    std::slice::from_raw_parts(guard.as_ptr(), guard.len())
                };
                Ok((slice_ref, Some(LockGuard::RefCellVec(guard))))
            },
            Container::ArcMutexBox(value) => {
                let guard = value.lock()?;
                let slice_ref = unsafe {
                    // SAFETY: The slice reference is valid as long as the guard is held
                    std::slice::from_raw_parts(guard.as_ptr(), guard.len())
                };
                Ok((slice_ref, Some(LockGuard::MutexBox(guard))))
            },
            Container::ArcMutexVec(value) => {
                let guard = value.lock()?;
                let slice_ref = unsafe {
                    // SAFETY: The slice reference is valid as long as the guard is held
                    std::slice::from_raw_parts(guard.as_ptr(), guard.len())
                };
                Ok((slice_ref, Some(LockGuard::MutexVec(guard))))
            },
            Container::ArcRwLockBox(value) => {
                let guard = value.read()?;
                let slice_ref = unsafe {
                    // SAFETY: The slice reference is valid as long as the guard is held
                    std::slice::from_raw_parts(guard.as_ptr(), guard.len())
                };
                Ok((slice_ref, Some(LockGuard::RwLockBox(guard))))
            },
            Container::ArcRwLockVec(value) => {
                let guard = value.read()?;
                let slice_ref = unsafe {
                    // SAFETY: The slice reference is valid as long as the guard is held
                    std::slice::from_raw_parts(guard.as_ptr(), guard.len())
                };
                Ok((slice_ref, Some(LockGuard::RwLockVec(guard))))
            },
        }
    }
}

// --- Additional utility methods ---

impl<'a, T: Clone> Container<'a, T> {
    /// Returns the length of the container
    pub fn len(&self) -> usize {
        match self {
            // ----- Simple Variants -----
            Container::Empty => 0,
            Container::Box(value) => value.len(),
            Container::Vec(value) => value.len(),
            Container::Ref(value) => value.len(),
            Container::Cow(value) => value.len(),
            Container::Rc(value) => value.len(),
            Container::Arc(value) => value.len(),
            // ----- Compount Variants -----
            Container::RefBox(value) => value.len(),
            Container::RefVec(value) => value.len(),
            Container::CowBox(value) => value.len(),
            Container::CowVec(value) => value.len(),
            Container::RcBox(value) => value.len(),
            Container::RcVec(value) => value.len(),
            Container::ArcBox(value) => value.len(),
            Container::ArcVec(value) => value.len(),
            // ----- Mutating Variants -----
            Container::RcRefCellBox(value) => value.borrow().len(),
            Container::RcRefCellVec(value) => value.borrow().len(),
            Container::ArcMutexBox(value) => value.lock().unwrap().len(),
            Container::ArcMutexVec(value) => value.lock().unwrap().len(),
            Container::ArcRwLockBox(value) => value.read().unwrap().len(),
            Container::ArcRwLockVec(value) => value.read().unwrap().len(),
        }
    }

    /// Returns true if the container is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Converts the container to an owned Vec<T>
    pub fn to_vec(&self) -> Vec<T> {
        match self {
            // ----- Simple Variants -----
            Container::Empty => Vec::new(),
            Container::Box(value) => value.to_vec(),
            Container::Vec(value) => value.clone(),
            Container::Ref(value) => value.to_vec(),
            Container::Cow(value) => value.to_vec(),
            Container::Rc(value) => value.to_vec(),
            Container::Arc(value) => value.to_vec(),
            // ----- Compount Variants -----
            Container::RefBox(value) => value.to_vec(),
            Container::RefVec(value) => value.to_vec(),
            Container::CowBox(value) => value.to_vec(),
            Container::CowVec(value) => value.to_vec(),
            Container::RcBox(value) => value.to_vec(),
            Container::RcVec(value) => (**value).clone(),
            Container::ArcBox(value) => value.to_vec(),
            Container::ArcVec(value) => (**value).clone(),
            // ----- Mutating Variants -----
            Container::RcRefCellBox(value) => value.borrow().to_vec(),
            Container::RcRefCellVec(value) => value.borrow().clone(),
            Container::ArcMutexBox(value) => value.lock().unwrap().to_vec(),
            Container::ArcMutexVec(value) => value.lock().unwrap().clone(),
            Container::ArcRwLockBox(value) => value.read().unwrap().to_vec(),
            Container::ArcRwLockVec(value) => value.read().unwrap().clone(),
        }
    }
}