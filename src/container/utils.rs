use super::core::Container;
use std::sync::Arc;
use std::borrow::Cow;
#[cfg(not(feature = "no-rc"))]
use std::rc::Rc;
#[cfg(all(feature = "locking", not(feature = "no-rc")))]
use std::cell::RefCell;
#[cfg(feature = "locking")]
use std::sync::{Mutex,RwLock};

// --- From Raw ---

// ----- Simple Variants -----
impl<T: Clone> From<(/* Empty */)>  for Container<T> {fn from((): ()) -> Self {Container::Empty}}
impl<T: Clone> From<Box<[T]>> for Container<T> {fn from(value: Box<[T]>) -> Self {Container::Box(value)}}
impl<T: Clone> From<Vec<T>> for Container<T> {fn from(value: Vec<T>) -> Self {Container::Vec(value)}}
impl<T: Clone> From<&'static [T]> for Container<T> {fn from(value: &'static [T]) -> Self {Container::Ref(value)}}
impl<T: Clone> From<Cow<'static, [T]>> for Container<T> {fn from(value: Cow<'static, [T]>) -> Self {Container::Cow(value)}}
#[cfg(not(feature = "no-rc"))]
impl<T: Clone> From<Rc<[T]>> for Container<T> {fn from(value: Rc<[T]>) -> Self {Container::Rc(value)}}
impl<T: Clone> From<Arc<[T]>> for Container<T> {fn from(value: Arc<[T]>) -> Self {Container::Arc(value)}}
// ----- Compound Variants -----
impl<T: Clone> From<&'static Box<[T]>> for Container<T> {fn from(value: &'static Box<[T]>) -> Self {Container::RefBox(value)}}
impl<T: Clone> From<&'static Vec<T>> for Container<T> {fn from(value: &'static Vec<T>) -> Self {Container::RefVec(value)}}
impl<T: Clone> From<Cow<'static, Box<[T]>>> for Container<T> {fn from(value: Cow<'static, Box<[T]>>) -> Self {Container::CowBox(value)}}
impl<T: Clone> From<Cow<'static, Vec<T>>> for Container<T> {fn from(value: Cow<'static, Vec<T>>) -> Self {Container::CowVec(value)}}
#[cfg(not(feature = "no-rc"))]
impl<T: Clone> From<Rc<Box<[T]>>> for Container<T> {fn from(value: Rc<Box<[T]>>) -> Self {Container::RcBox(value)}}
#[cfg(not(feature = "no-rc"))]
impl<T: Clone> From<Rc<Vec<T>>> for Container<T> {fn from(value: Rc<Vec<T>>) -> Self {Container::RcVec(value)}}
impl<T: Clone> From<Arc<Box<[T]>>> for Container<T> {fn from(value: Arc<Box<[T]>>) -> Self {Container::ArcBox(value)}}
impl<T: Clone> From<Arc<Vec<T>>> for Container<T> {fn from(value: Arc<Vec<T>>) -> Self {Container::ArcVec(value)}}
// ----- Locking Variants -----
#[cfg(all(feature = "locking", not(feature = "no-rc")))]
impl<T: Clone> From<Rc<RefCell<Box<[T]>>>> for Container<T> {fn from(value: Rc<RefCell<Box<[T]>>>) -> Self {Container::RcRefCellBox(value)}}
#[cfg(all(feature = "locking", not(feature = "no-rc")))]
impl<T: Clone> From<Rc<RefCell<Vec<T>>>> for Container<T> {fn from(value: Rc<RefCell<Vec<T>>>) -> Self {Container::RcRefCellVec(value)}}
#[cfg(feature = "locking")]
impl<T: Clone> From<Arc<Mutex<Box<[T]>>>> for Container<T> {fn from(value: Arc<Mutex<Box<[T]>>>) -> Self {Container::ArcMutexBox(value)}}
#[cfg(feature = "locking")]
impl<T: Clone> From<Arc<Mutex<Vec<T>>>> for Container<T> {fn from(value: Arc<Mutex<Vec<T>>>) -> Self {Container::ArcMutexVec(value)}}
#[cfg(feature = "locking")]
impl<T: Clone> From<Arc<RwLock<Box<[T]>>>> for Container<T> {fn from(value: Arc<RwLock<Box<[T]>>>) -> Self {Container::ArcRwLockBox(value)}}
#[cfg(feature = "locking")]
impl<T: Clone> From<Arc<RwLock<Vec<T>>>> for Container<T> {fn from(value: Arc<RwLock<Vec<T>>>) -> Self {Container::ArcRwLockVec(value)}}


// --- Clone ---

// Clone for Container<T> where T is Clone
impl<T: Clone> Clone for Container<T> {
    /// Creates a new container by cheaply copying a smart pointer if possible. 
    fn clone(&self) -> Self {
        match self {
            // ----- Simple Variants -----
            Container::Empty => Container::Empty,
            Container::Box(value) => Container::Box(value.clone()),
            Container::Vec(value) => Container::Vec(value.clone()),
            Container::Ref(value) => Container::Ref(value),
            Container::Cow(value) => Container::Cow(value.clone()),
            #[cfg(not(feature = "no-rc"))]
            Container::Rc(value) => Container::Rc(value.clone()),
            Container::Arc(value) => Container::Arc(value.clone()),
            // ----- Compound Variants -----
            Container::RefBox(value) => Container::RefBox(value),
            Container::RefVec(value) => Container::RefVec(value),
            Container::CowBox(value) => Container::CowBox(value.clone()),
            Container::CowVec(value) => Container::CowVec(value.clone()),
            #[cfg(not(feature = "no-rc"))]
            Container::RcBox(value) => Container::RcBox(value.clone()),
            #[cfg(not(feature = "no-rc"))]
            Container::RcVec(value) => Container::RcVec(value.clone()),
            Container::ArcBox(value) => Container::ArcBox(value.clone()),
            Container::ArcVec(value) => Container::ArcVec(value.clone()),
            // ----- Locking Variants -----
            #[cfg(all(feature = "locking", not(feature = "no-rc")))]
            Container::RcRefCellBox(value) => Container::RcRefCellBox(value.clone()),
            #[cfg(all(feature = "locking", not(feature = "no-rc")))]
            Container::RcRefCellVec(value) => Container::RcRefCellVec(value.clone()),
            #[cfg(feature = "locking")]
            Container::ArcMutexBox(value) => Container::ArcMutexBox(value.clone()),
            #[cfg(feature = "locking")]
            Container::ArcMutexVec(value) => Container::ArcMutexVec(value.clone()),
            #[cfg(feature = "locking")]
            Container::ArcRwLockBox(value) => Container::ArcRwLockBox(value.clone()),
            #[cfg(feature = "locking")]
            Container::ArcRwLockVec(value) => Container::ArcRwLockVec(value.clone()),
        }
    }
}

// --- Additional utility methods ---
#[cfg(not(feature = "locking"))]
impl<T: Clone> Container<T> {
    /// Returns the length of the container.
    /// Only available if locking variants are disabled.
    #[must_use]
    pub fn len(&self) -> usize {
        self.as_ref().len()
    }

    /// Returns true if the container is empty.
    /// Only available if locking variants are disabled.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Converts the container to an Owned `Vec<T>` by cloning each `item: T`.
    /// Only available if locking variants are disabled.
    #[must_use]
    pub fn to_vec(&self) -> Vec<T> {
        self.as_ref().iter().map(|t: &T|{t.clone()}).collect::<Vec<T>>()
    }

    /// Converts the container to an Owned `Container<T>` by cloning each `item: T`.
    /// Only available if locking variants are disabled.
    #[must_use]
    pub fn to_owned(&self) -> Container<T> {
        Container::Vec(self.to_vec())
    }
}

// ----- Serialize -----
#[cfg(feature = "serializable")]
mod serialize {
    use super::super::Container;
    use super::super::SafeBorrow;
    use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeSeq};
    /// Serialize a Borrow. 
    impl<T: Serialize + Clone> Serialize for SafeBorrow<'_, T> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer {
                let mut seq = serializer.serialize_seq(Some(self.len()))?;
                for e in self.as_ref() {
                    seq.serialize_element(e)?;
                }
                seq.end()    
        }
    }
    /// Serialize a Container by borrowing.
    /// Only available if locking variants are disabled.
    #[cfg(not(feature = "locking"))]
    impl<T: Serialize + Clone> Serialize for Container<T> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer {
                let mut seq = serializer.serialize_seq(Some(self.len()))?;
                for e in self.as_ref() {
                    seq.serialize_element(e)?;
                }
                seq.end()
        }
    }
    /// Deserialize into a Container.
    impl<'de, T: Deserialize<'de> + Clone> Deserialize<'de> for Container<T> {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            let v = Vec::<T>::deserialize(d)?;
            Ok(Container::Vec(v))
        }
    }
}