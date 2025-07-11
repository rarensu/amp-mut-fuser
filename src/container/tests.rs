// This file contains all unit tests for the Container enum
// and its trait implementations.

// Note: When container/mod.rs is set up with `pub use super::core::Container;`
// and `#[cfg(test)] mod tests;`, the tests here can use `use crate::container::Container;`
// or `use super::Container;` if `mod.rs` re-exports `Container` from `core`.
// For now, assuming `crate::container::Container` will be the path once `mod.rs` is set up.
// If not, `super::core::Container` or `crate::container::core::Container` might be needed.

use crate::container::Container; // Assuming Container is re-exported by src/container/mod.rs
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
struct MyStruct {
    id: i32,
    data: String,
}

// --- Tests for Container<'a, u8> ---
#[test]
fn container_clone_slice() {
    let data_vec: Vec<u8> = vec![1, 2, 3];
    let container1: Container<'_, u8> = Container::from(data_vec.clone()); // This will be Container::Vec
    let container2 = container1.clone();
    assert_eq!(container1.as_ref(), container2.as_ref());
    match (&container1, &container2) {
        (Container::Vec(v1), Container::Vec(v2)) => { // Expect Vec after cloning Vec
            assert_ne!(v1.as_ptr(), v2.as_ptr(), "Cloned Vec<u8> should have different pointers");
        }
        _ => panic!("Expected Vec variants for Container::from(Vec<u8>) clone"),
    }

    let borrowed_slice: &[u8] = &[4,5,6];
    let c_borrowed1 : Container<'_, u8> = Container::Ref(borrowed_slice);
    let c_borrowed2 = c_borrowed1.clone();
    assert_eq!(c_borrowed1.as_ref(), c_borrowed2.as_ref());
    assert_eq!(c_borrowed1.as_ref().as_ptr(), c_borrowed2.as_ref().as_ptr());


    let shared_slice_arc: Arc<[u8]> = Arc::new([7,8,9]);
    let c_shared1 : Container<'_, u8> = Container::Arc(shared_slice_arc.clone());
    let c_shared2 = c_shared1.clone();
    if let Container::Arc(s2_arc) = &c_shared2 {
        assert!(Arc::ptr_eq(s2_arc, &shared_slice_arc));
    } else { panic!("Expected shared after clone"); }
}

#[test]
fn container_deref_slice() {
    let data: &[u8] = &[1, 2, 3, 4, 5];
    let container: Container<'_, u8> = Container::from(data);
    assert_eq!(container.len(), 5); // Test Deref to [u8]
    assert_eq!(container[0], 1);
}

#[test]
fn test_borrowed_variants() {
    let data_slice: &'static [u8] = &[1, 2, 3];
    let data_box: Box<[u8]> = Box::new([4, 5, 6]);
    let data_vec: Vec<u8> = vec![7, 8, 9];

    // Container::Ref
    let container_ref = Container::Ref(data_slice);
    assert_eq!(container_ref.as_ref(), data_slice);

    // Container::Cow (borrowed)
    let container_cow_borrowed = Container::Cow(std::borrow::Cow::Borrowed(data_slice));
    assert_eq!(container_cow_borrowed.as_ref(), data_slice);

    // Container::RefBox
    let container_ref_box = Container::RefBox(&data_box);
    assert_eq!(container_ref_box.as_ref(), data_box.as_ref());

    // Container::RefVec
    let container_ref_vec = Container::RefVec(&data_vec);
    assert_eq!(container_ref_vec.as_ref(), data_vec.as_slice());

    // Container::CowBox (borrowed)
    let container_cow_box_borrowed = Container::CowBox(std::borrow::Cow::Borrowed(&data_box));
    assert_eq!(container_cow_box_borrowed.as_ref(), data_box.as_ref());

    // Container::CowVec (borrowed)
    let container_cow_vec_borrowed = Container::CowVec(std::borrow::Cow::Borrowed(&data_vec));
    assert_eq!(container_cow_vec_borrowed.as_ref(), data_vec.as_slice());
}

#[test]
fn test_owned_variants() {
    let data_box_orig: Box<[u8]> = Box::new([1, 2, 3]);
    let data_vec_orig: Vec<u8> = vec![4, 5, 6];
    let data_rc_orig: std::rc::Rc<[u8]> = std::rc::Rc::new([7, 8, 9]);
    let data_arc_orig: Arc<[u8]> = Arc::new([10, 11, 12]);

    // Container::Box
    let container_box = Container::from(data_box_orig.clone());
    assert_eq!(container_box.try_as_ref().unwrap(), data_box_orig.as_ref());

    // Container::Vec
    let container_vec = Container::from(data_vec_orig.clone());
    assert_eq!(container_vec.try_as_ref().unwrap(), data_vec_orig.as_slice());

    // Container::Cow (owned from Vec)
    let container_cow_owned_vec = Container::Cow(std::borrow::Cow::Owned(data_vec_orig.clone()));
    assert_eq!(container_cow_owned_vec.try_as_ref().unwrap(), data_vec_orig.as_slice());

    // Container::Cow (owned from Box) - Note: Cow doesn't directly take Box<[T]>, so we convert to Vec<T> first for owned Cow
    let container_cow_owned_box_as_vec = Container::Cow(std::borrow::Cow::Owned(data_box_orig.to_vec()));
    assert_eq!(container_cow_owned_box_as_vec.try_as_ref().unwrap(), data_box_orig.as_ref());

    // Container::Rc
    let container_rc = Container::from(data_rc_orig.clone());
    assert_eq!(container_rc.try_as_ref().unwrap(), data_rc_orig.as_ref());

    // Container::Arc
    let container_arc = Container::from(data_arc_orig.clone());
    assert_eq!(container_arc.try_as_ref().unwrap(), data_arc_orig.as_ref());

    // Container::RcBox
    let container_rc_box = Container::RcBox(std::rc::Rc::new(data_box_orig.clone()));
    assert_eq!(container_rc_box.try_as_ref().unwrap(), data_box_orig.as_ref());

    // Container::RcVec
    let container_rc_vec = Container::RcVec(std::rc::Rc::new(data_vec_orig.clone()));
    assert_eq!(container_rc_vec.try_as_ref().unwrap(), data_vec_orig.as_slice());

    // Container::ArcBox
    let container_arc_box = Container::ArcBox(Arc::new(data_box_orig.clone()));
    assert_eq!(container_arc_box.try_as_ref().unwrap(), data_box_orig.as_ref());

    // Container::ArcVec
    let container_arc_vec = Container::ArcVec(Arc::new(data_vec_orig.clone()));
    assert_eq!(container_arc_vec.try_as_ref().unwrap(), data_vec_orig.as_slice());
}

#[test]
fn test_mutating_variants_read() {
    let data_box_orig: Box<[u8]> = Box::new([1, 2, 3]);
    let data_vec_orig: Vec<u8> = vec![4, 5, 6];

    // Container::RcRefCellBox
    let rc_ref_cell_box = std::rc::Rc::new(std::cell::RefCell::new(data_box_orig.clone()));
    let container_rc_ref_cell_box: Container<'static, u8> = Container::from(rc_ref_cell_box.clone());
    match container_rc_ref_cell_box.get_slice() {
        Ok(guard) => assert_eq!(guard.0, data_box_orig.as_ref()),
        Err(_) => panic!("Expected Ok for RcRefCellBox get_slice"),
    };

    // Container::RcRefCellVec
    let rc_ref_cell_vec = std::rc::Rc::new(std::cell::RefCell::new(data_vec_orig.clone()));
    let container_rc_ref_cell_vec: Container<'static, u8> = Container::from(rc_ref_cell_vec.clone());
    match container_rc_ref_cell_vec.get_slice() {
        Ok(guard) => assert_eq!(guard.0, data_vec_orig.as_slice()),
        Err(_) => panic!("Expected Ok for RcRefCellVec get_slice"),
    };

    // Container::ArcMutexBox
    let arc_mutex_box = Arc::new(std::sync::Mutex::new(data_box_orig.clone()));
    let container_arc_mutex_box: Container<'static, u8> = Container::from(arc_mutex_box.clone());
    match container_arc_mutex_box.get_slice() {
        Ok(guard) => assert_eq!(guard.0, data_box_orig.as_ref()),
        Err(_) => panic!("Expected Ok for ArcMutexBox get_slice"),
    };

    // Container::ArcMutexVec
    let arc_mutex_vec = Arc::new(std::sync::Mutex::new(data_vec_orig.clone()));
    let container_arc_mutex_vec: Container<'static, u8> = Container::from(arc_mutex_vec.clone());
    match container_arc_mutex_vec.get_slice() {
        Ok(guard) => assert_eq!(guard.0, data_vec_orig.as_slice()),
        Err(_) => panic!("Expected Ok for ArcMutexVec get_slice"),
    };

    // Container::ArcRwLockBox
    let arc_rw_lock_box = Arc::new(std::sync::RwLock::new(data_box_orig.clone()));
    let container_arc_rw_lock_box: Container<'static, u8> = Container::from(arc_rw_lock_box.clone());
    match container_arc_rw_lock_box.get_slice() {
        Ok(guard) => assert_eq!(guard.0, data_box_orig.as_ref()),
        Err(_) => panic!("Expected Ok for ArcRwLockBox get_slice"),
    };

    // Container::ArcRwLockVec
    let arc_rw_lock_vec = Arc::new(std::sync::RwLock::new(data_vec_orig.clone()));
    let container_arc_rw_lock_vec: Container<'static, u8> = Container::from(arc_rw_lock_vec.clone());
    match container_arc_rw_lock_vec.get_slice() {
        Ok(guard) => assert_eq!(guard.0, data_vec_orig.as_slice()),
        Err(_) => panic!("Expected Ok for ArcRwLockVec get_slice"),
    };
}

#[test]
fn test_mutating_variants_try_as_ref_error() {
    let data_box_orig: Box<[u8]> = Box::new([1, 2, 3]);
    let data_vec_orig: Vec<u8> = vec![4, 5, 6];

    // Container::RcRefCellBox
    let rc_ref_cell_box = std::rc::Rc::new(std::cell::RefCell::new(data_box_orig.clone()));
    let container_rc_ref_cell_box: Container<'static, u8> = Container::from(rc_ref_cell_box);
    assert_eq!(container_rc_ref_cell_box.try_as_ref(), Err("Attempted to get a reference from Container::RcRefCellBox without the proper lock."));

    // Container::RcRefCellVec
    let rc_ref_cell_vec = std::rc::Rc::new(std::cell::RefCell::new(data_vec_orig.clone()));
    let container_rc_ref_cell_vec: Container<'static, u8> = Container::from(rc_ref_cell_vec);
    assert_eq!(container_rc_ref_cell_vec.try_as_ref(), Err("Attempted to get a reference from Container::RcRefCellVec without the proper lock."));

    // Container::ArcMutexBox
    let arc_mutex_box = Arc::new(std::sync::Mutex::new(data_box_orig.clone()));
    let container_arc_mutex_box: Container<'static, u8> = Container::from(arc_mutex_box);
    assert_eq!(container_arc_mutex_box.try_as_ref(), Err("Attempted to get a reference from Container::ArcMutexBox without the proper lock."));

    // Container::ArcMutexVec
    let arc_mutex_vec = Arc::new(std::sync::Mutex::new(data_vec_orig.clone()));
    let container_arc_mutex_vec: Container<'static, u8> = Container::from(arc_mutex_vec);
    assert_eq!(container_arc_mutex_vec.try_as_ref(), Err("Attempted to get a reference from Container::ArcMutexVec without the proper lock."));

    // Container::ArcRwLockBox
    let arc_rw_lock_box = Arc::new(std::sync::RwLock::new(data_box_orig.clone()));
    let container_arc_rw_lock_box: Container<'static, u8> = Container::from(arc_rw_lock_box);
    assert_eq!(container_arc_rw_lock_box.try_as_ref(), Err("Attempted to get a reference from Container::ArcRwLockBox without the proper lock."));

    // Container::ArcRwLockVec
    let arc_rw_lock_vec = Arc::new(std::sync::RwLock::new(data_vec_orig.clone()));
    let container_arc_rw_lock_vec: Container<'static, u8> = Container::from(arc_rw_lock_vec);
    assert_eq!(container_arc_rw_lock_vec.try_as_ref(), Err("Attempted to get a reference from Container::ArcRwLockVec without the proper lock."));
}