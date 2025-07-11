use std::sync::{Arc,Mutex,RwLock};
use std::rc::Rc;
use std::borrow::Cow;
use std::cell::RefCell;

/// A generic container enum that provides flexible ownership models for unsized data types.
/// If there is a borrow, its lifetime is 'a (most likely 'static).
#[derive(Debug)]
pub enum Container<'a, T: Clone> {
    // ----- Simple Variants -----
    /// No data.
    Empty,
    /// An owned, fixed-size, heap-allocated slice.
    Box(Box<[T]>),
    /// An owned, growable, heap-allocated vector.
    Vec(Vec<T>),
    /// A borrowed slice.
    Ref(&'a [T]),
    /// A borrowed slice with copy-on-write.
    Cow(Cow<'a, [T]>),
    /// A reusable, fixed-size slice.
    Rc(Rc<[T]>),
    /// A shared, fixed-size slice.
    Arc(Arc<[T]>),
    // ----- Compount Variants -----
    /// A borrowed, fixed-size, heap-allocated slice. 
    RefBox(&'a Box<[T]>),
    /// A borrowed, immutable, heap-allocated vector. 
    RefVec(&'a Vec<T>),
    /// A borrowed, fixed-size, heap-allocated vector, with copy-on-write. 
    CowBox(Cow<'a, Box<[T]>>),
    /// A borrowed, immutable, heap-allocated vactor, with copy-on-write.
    CowVec(Cow<'a, Vec<T>>),
    /// A reusable, fixed-size, heap-allocated slice.
    RcBox(Rc<Box<[T]>>),
    /// A reusable, immutable, heap-allocated vector.
    RcVec(Rc<Vec<T>>),
    /// A shared, fixed-size, heap-allocated slice.
    ArcBox(Arc<Box<[T]>>),
    /// A shared, immutable, heap-allocated vector.
    ArcVec(Arc<Vec<T>>),
    // ----- Locking Variants -----
    /// A reusable, replaceable, heap-allocated slice.
    RcRefCellBox(Rc<RefCell<Box<[T]>>>),
    /// A reusable, growable, heap-allocated vector.
    RcRefCellVec(Rc<RefCell<Vec<T>>>),
    /// A shared, fixed-size, replacable, heap-allocated slice.
    ArcMutexBox(Arc<Mutex<Box<[T]>>>),
    /// A shared, growable, heap-allocated vector.
    ArcMutexVec(Arc<Mutex<Vec<T>>>),
    /// A shared, fixed-size, replacable, heap-allocated slide with multiple readers.
    ArcRwLockBox(Arc<RwLock<Box<[T]>>>),
    /// A shared, growable, heap-allocated vector with multiple readers.
    ArcRwLockVec(Arc<RwLock<Vec<T>>>),
}

// --- Generic From<T> implementations ---

// ----- Simple Variants -----
impl<'a, T: Clone> From<(/* Empty */)>  for Container<'a, T> {fn from(_: ()) -> Self {Container::Empty}}
impl<'a, T: Clone> From<Box<[T]>> for Container<'a, T> {fn from(value: Box<[T]>) -> Self {Container::Box(value)}}
impl<'a, T: Clone> From<Vec<T>> for Container<'a, T> {fn from(value: Vec<T>) -> Self {Container::Vec(value)}}
impl<'a, T: Clone> From<&'a [T]> for Container<'a, T> {fn from(value: &'a [T]) -> Self {Container::Ref(value)}}
impl<'a, T: Clone> From<Cow<'a, [T]>> for Container<'a, T> {fn from(value: Cow<'a, [T]>) -> Self {Container::Cow(value)}}
impl<'a, T: Clone> From<Rc<[T]>> for Container<'a, T> {fn from(value: Rc<[T]>) -> Self {Container::Rc(value)}}
impl<'a, T: Clone> From<Arc<[T]>> for Container<'a, T> {fn from(value: Arc<[T]>) -> Self {Container::Arc(value)}}
// ----- Compound Variants -----
impl<'a, T: Clone> From<&'a Box<[T]>> for Container<'a, T> {fn from(value: &'a Box<[T]>) -> Self {Container::RefBox(value)}}
impl<'a, T: Clone> From<&'a Vec<T>> for Container<'a, T> {fn from(value: &'a Vec<T>) -> Self {Container::RefVec(value)}}
impl<'a, T: Clone> From<Cow<'a, Box<[T]>>> for Container<'a, T> {fn from(value: Cow<'a, Box<[T]>>) -> Self {Container::CowBox(value)}}
impl<'a, T: Clone> From<Cow<'a, Vec<T>>> for Container<'a, T> {fn from(value: Cow<'a, Vec<T>>) -> Self {Container::CowVec(value)}}
impl<'a, T: Clone> From<Rc<Box<[T]>>> for Container<'a, T> {fn from(value: Rc<Box<[T]>>) -> Self {Container::RcBox(value)}}
impl<'a, T: Clone> From<Rc<Vec<T>>> for Container<'a, T> {fn from(value: Rc<Vec<T>>) -> Self {Container::RcVec(value)}}
impl<'a, T: Clone> From<Arc<Box<[T]>>> for Container<'a, T> {fn from(value: Arc<Box<[T]>>) -> Self {Container::ArcBox(value)}}
impl<'a, T: Clone> From<Arc<Vec<T>>> for Container<'a, T> {fn from(value: Arc<Vec<T>>) -> Self {Container::ArcVec(value)}}
// ----- Locking Variants -----
impl<'a, T: Clone> From<Rc<RefCell<Box<[T]>>>> for Container<'a, T> {fn from(value: Rc<RefCell<Box<[T]>>>) -> Self {Container::RcRefCellBox(value)}}
impl<'a, T: Clone> From<Rc<RefCell<Vec<T>>>> for Container<'a, T> {fn from(value: Rc<RefCell<Vec<T>>>) -> Self {Container::RcRefCellVec(value)}}
impl<'a, T: Clone> From<Arc<Mutex<Box<[T]>>>> for Container<'a, T> {fn from(value: Arc<Mutex<Box<[T]>>>) -> Self {Container::ArcMutexBox(value)}}
impl<'a, T: Clone> From<Arc<Mutex<Vec<T>>>> for Container<'a, T> {fn from(value: Arc<Mutex<Vec<T>>>) -> Self {Container::ArcMutexVec(value)}}
impl<'a, T: Clone> From<Arc<RwLock<Box<[T]>>>> for Container<'a, T> {fn from(value: Arc<RwLock<Box<[T]>>>) -> Self {Container::ArcRwLockBox(value)}}
impl<'a, T: Clone> From<Arc<RwLock<Vec<T>>>> for Container<'a, T> {fn from(value: Arc<RwLock<Vec<T>>>) -> Self {Container::ArcRwLockVec(value)}}