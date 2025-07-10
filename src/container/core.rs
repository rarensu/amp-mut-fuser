use std::sync::{Arc,Mutex,RwLock};
use std::borrow::Cow;
use std::ops::Deref;

/// A generic container enum that provides flexible ownership for various data types.
/// If there is a borrow, its lifetime 'a is most likely 'static.
#[derive(Debug)]
pub enum Container<'a, T: Clone> {
    /// No data.
    Empty,
    /// A borrowed slice.
    Ref(&'a [T]),
    /// A borrowed slice with copy-on-write.
    Cow(Cow<'a, [T]>),
    /// An owned, fixed-size, heap-allocated slice.
    Box(Box<[T]>),
    /// An owned, growable, heap-allocated vector.
    Vec(Vec<T>),
    /// A shared, fixed-size slice.
    Arc(Arc<[T]>),
    /// A shared, immutable, heap-allocated vector.
    ArcVec(Arc<Vec<T>>),
    /// A shared, growable, heap-allocated vector.
    ArcMutexVec(Arc<Mutex<Vec<T>>>),
    /// A shared, growable, heap-allocated vector with multiple readers.
    ArcRwLockVec(Arc<RwLock<Vec<T>>>),
    /// A borrowed, fixed-size, heap-allocated vector, with copy-on-write. 
    CowBox(Cow<'a, Box<[T]>>),
    /// A borrowed, immutable, heap-allocated slice, with copy-on-write.
    CowVec(Cow<'a, Vec<T>>),
    /// An owned, immutable, heap-allocated vector of borrowed references.
    VecRef(Vec<&'a T>),
    /// An owned, growable, twice-heap-allocated vector.
    BoxVec(Box<Vec<T>>),
    /// An owned, growable, heap-allocated vector of borrowed references with copy-on-write.
    VecCow(Vec<Cow<'a, T>>),
    /// A shared, growable, heap-allocated vector of borrowed references.
    ArcMutexVecRef(Arc<Mutex<Vec<&'a T>>>),
    /// A shared, growable, heap-allocated vector of borrowed references with copy-on-write.
    ArcMutexVecCow(Arc<Mutex<Vec<Cow<'a, T>>>>),
    /// A shared, growable, heap-allocated vector of borrowed references with multiple readers.
    ArcRwLockVecRef(Arc<RwLock<Vec<&'a T>>>),
    /// A shared, growable, heap-allocated vector of borrowed references with copy-on-write and multiple readers.
    ArcRwLockVecCow(Arc<RwLock<Vec<Cow<'a, T>>>>),

}

// --- Generic From implementations ---

impl<'a, T: Clone> From<()> for Container<'a, T> {fn from(_value: ()) -> Self {Container::Empty}}
impl<'a, T: Clone> From<&'a [T]> for Container<'a, T> {fn from(value: &'a [T]) -> Self {Container::Ref(value)}}
impl<'a, T: Clone> From<Cow<'a, [T]>> for Container<'a, T> {fn from(value: Cow<'a, [T]>) -> Self {Container::Cow()}}
impl<'a, T: Clone> From<Box<[T]>> for Container<'a, T> {fn from(value: Box<[T]>) -> Self {Container::Box(value)}}
impl<'a, T: Clone> From<Vec<T>> for Container<'a, T> {fn from(value: Vec<T>) -> Self {Container::Vec()}}
impl<'a, T: Clone> From<Arc<[T]>> for Container<'a, T> {fn from(value: Arc<[T]>) -> Self {Container::Arc(value)}}
impl<'a, T: Clone> From<Arc<Vec<T>>> for Container<'a, T> {fn from(value: Arc<Vec<T>>) -> Self {Container::ArcVec()}}
impl<'a, T: Clone> From<Arc<Mutex<Vec<T>>>> for Container<'a, T> {fn from(value: Arc<Mutex<Vec<T>>>) -> Self {Container::ArcMutexVec()}}
impl<'a, T: Clone> From<Arc<RwLock<Vec<T>>>> for Container<'a, T> {fn from(value: Arc<RwLock<Vec<T>>>) -> Self {Container::ArcRwLockVec()}}
impl<'a, T: Clone> From<Cow<'a, Box<[T]>>> for Container<'a, T> {fn from(value: Cow<'a, Box<[T]>>) -> Self {Container::CowBox()}}
impl<'a, T: Clone> From<Cow<'a, Vec<T>>> for Container<'a, T> {fn from(value: Cow<'a, Vec<T>>) -> Self {Container::CowVec()}}
impl<'a, T: Clone> From<Vec<&'a T>> for Container<'a, T> {fn from(value: Vec<&'a T>) -> Self {Container::VecRef()}}
impl<'a, T: Clone> From<Box<Vec<T>>> for Container<'a, T> {fn from(value: Box<Vec<T>>) -> Self {Container::BoxVec()}}
impl<'a, T: Clone> From<Vec<Cow<'a, T>>> for Container<'a, T> {fn from(value: Vec<Cow<'a, T>>) -> Self {Container::VecCow()}}
impl<'a, T: Clone> From<Arc<Mutex<Vec<&'a T>>>> for Container<'a, T> {fn from(value: Arc<Mutex<Vec<&'a T>>>) -> Self {Container::ArcMutexVecRef()}}
impl<'a, T: Clone> From<Arc<Mutex<Vec<Cow<'a, T>>>>> for Container<'a, T> {fn from(value: Arc<Mutex<Vec<Cow<'a, T>>>>) -> Self {Container::ArcMutexVecCow()}}
impl<'a, T: Clone> From<Arc<RwLock<Vec<&'a T>>>> for Container<'a, T> {fn from(value: Arc<RwLock<Vec<&'a T>>>) -> Self {Container::ArcRwLockVecRef()}}
impl<'a, T: Clone> From<Arc<RwLock<Vec<Cow<'a, T>>>>> for Container<'a, T> {fn from(value: Arc<RwLock<Vec<Cow<'a, T>>>>) -> Self {Container::ArcRwLockVecCow()}}

// --- AsRef and Deref implementations ---

impl<'a, T: Clone> AsRef<[T]> for Container<'a, T> {
    fn as_ref(&self) -> &[T] {
        match self {
            Container::Empty => &[], // the 'static zero-length slice of type T
            Container::Ref(value) => value,
            Container::Cow(value)=>,
            Container::Box(value) => value.as_ref(),
            Container::Vec(value)=>,
            Container::Arc(value) => value.as_ref(),
            Container::ArcVec(value)=>,
            Container::ArcMutexVec(value)=>,
            Container::ArcRwLockVec(value)=>,
            Container::CowBox(value)=>,
            Container::CowVec(value)=>,
            Container::VecRef(value)=>,
            Container::BoxVec(value)=>,
            Container::VecCow(value)=>,
            Container::ArcMutexVecRef(value)=>,
            Container::ArcMutexVecCow(value)=>,
            Container::ArcRwLockVecRef(value)=>,
            Container::ArcRwLockVecCow(value)=>,
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
            Container::Empty => Container::Empty,
            Container::Ref(value) => Container::Ref(value),
            Container::Cow(value)=> Container::Cow(),
            Container::Box(value) => Container::Box(value.clone()),
            Container::Vec(value)=> Container::Vec(),
            Container::Arc(value) => Container::Arc(Arc::clone(value)),
            Container::ArcVec(value)=> Container::ArcVec(),
            Container::ArcMutexVec(value)=> Container::ArcMutexVec(),
            Container::ArcRwLockVec(value)=> Container::ArcRwLockVec(),
            Container::CowBox(value)=> Container::CowBox(),
            Container::CowVec(value)=> Container::CowVec(),
            Container::VecRef(value)=> Container::VecRef(),
            Container::BoxVec(value)=> Container::BoxVec(),
            Container::VecCow(value)=> Container::VecCow(),
            Container::ArcMutexVecRef(value)=> Container::ArcMutexVecRef(),
            Container::ArcMutexVecCow(value)=> Container::ArcMutexVecCow(),
            Container::ArcRwLockVecRef(value)=> Container::ArcRwLockVecRef(),
            Container::ArcRwLockVecCow(value)=> Container::ArcRwLockVecCow(),
        }
    }
}
