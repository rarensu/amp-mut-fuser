use super::core::Container;
use std::borrow::Cow;
#[cfg(all(feature = "locking", not(feature = "no-rc")))]
use std::cell::StaticCell;
#[cfg(not(feature = "no-rc"))]
use std::rc::Rc;
use std::sync::Arc;
#[cfg(feature = "locking")]
use std::sync::{Mutex, RwLock};

/* ----- From ----- */

macro_rules! impl_from {
    ($STRUCT:ty, $VARIANT:ident) => {
        impl<T: Clone> From<$STRUCT> for Container<T> {
            fn from(value: $STRUCT) -> Self {
                Container::$VARIANT(value)
            }
        }
    };
}

// Simple Variants
impl<T: Clone> From<()> for Container<T> {
    fn from((): ()) -> Self {
        Container::Empty
    }
}
impl_from!(Box<[T]>, Box);
impl_from!(Vec<T>, Vec);
impl_from!(&'static [T], Static);
impl_from!(Cow<'static, [T]>, Cow);
#[cfg(not(feature = "no-rc"))]
impl_from!(Rc<[T]>, Rc);
impl_from!(Arc<[T]>, Arc);
// Compound Variants
impl_from!(&'static Box<[T]>, StaticBox);
impl_from!(&'static Vec<T>, StaticVec);
impl_from!(Cow<'static, Box<[T]>>, CowBox);
impl_from!(Cow<'static, Vec<T>>, CowVec);
#[cfg(not(feature = "no-rc"))]
impl_from!(Rc<Box<[T]>>, RcBox);
#[cfg(not(feature = "no-rc"))]
impl_from!(Rc<Vec<T>>, RcVec);
impl_from!(Arc<Box<[T]>>, ArcBox);
impl_from!(Arc<Vec<T>>, ArcVec);
// Locking Variants
#[cfg(all(feature = "locking", not(feature = "no-rc")))]
impl_from!(Rc<RefCell<Box<[T]>>>, RcRefCellBox);
#[cfg(all(feature = "locking", not(feature = "no-rc")))]
impl_from!(Rc<RefCell<Vec<T>>>, RcRefCellVec);
#[cfg(feature = "locking")]
impl_from!(Arc<Mutex<Box<[T]>>>, ArcMutexBox);
#[cfg(feature = "locking")]
impl_from!(Arc<Mutex<Vec<T>>>, ArcMutexVec);
#[cfg(feature = "locking")]
impl_from!(Arc<RwLock<Box<[T]>>>, ArcRwLockBox);
#[cfg(feature = "locking")]
impl_from!(Arc<RwLock<Vec<T>>>, ArcRwLockVec);

/* ------ Clone ------ */

// Clone for Container<T> where T is Clone
impl<T: Clone> Clone for Container<T> {
    /// Creates a new container by cheaply copying a smart pointer if possible.
    fn clone(&self) -> Self {
        match self {
            // ----- Simple Variants -----
            Container::Empty => Container::Empty,
            Container::Box(value) => Container::Box(value.clone()),
            Container::Vec(value) => Container::Vec(value.clone()),
            Container::Static(value) => Container::Static(value),
            Container::Cow(value) => Container::Cow(value.clone()),
            #[cfg(not(feature = "no-rc"))]
            Container::Rc(value) => Container::Rc(value.clone()),
            Container::Arc(value) => Container::Arc(value.clone()),
            // ----- Compound Variants -----
            Container::StaticBox(value) => Container::StaticBox(value),
            Container::StaticVec(value) => Container::StaticVec(value),
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
        self.as_ref().to_vec()
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
            S: Serializer,
        {
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
            S: Serializer,
        {
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
