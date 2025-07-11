// This file contains unit tests for the Container enum
// and its trait implementations.

use crate::container::Container;
use std::sync::Arc;

#[cfg(test)]
mod u8 {
    use super::*;

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
    fn borrowed_variants() {
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
    fn owned_variants() {
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
    fn locking_variants_read() {
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
    fn locking_variants_try_as_ref_error() {
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
}

#[cfg(test)]
mod string {
    use crate::container::Container;
    #[cfg(unix)]
    use std::os::unix::ffi::OsStrExt;
    use crate::container::specialized::ToStringError;
    use std::ffi::{OsStr, OsString};
    use std::sync::{Arc, Mutex, RwLock};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::borrow::Cow;

    // --- Test Data ---
    const VALID_UTF8_STR: &str = "hello world";
    const VALID_UTF8_BYTES: &[u8] = b"hello world";
    // Invalid UTF-8 sequence (from Rust docs for from_utf8)
    const INVALID_UTF8_BYTES: &[u8] = &[0xf0, 0x90, 0x80];


    // --- Tests for From<String/&str> ---
    #[test]
    fn from_string_etc_for_container_u8() {
        let s = String::from(VALID_UTF8_STR);
        let container: Container<'_, u8> = Container::from(s.clone());
        assert_eq!(container.as_ref(), s.as_bytes());
        match container {
            Container::Vec(_) => {} // Expected
            _ => panic!("Expected Container::Vec from String"),
        }
   
        let s_ref: &str = VALID_UTF8_STR;
        let container: Container<'_, u8> = Container::from(s_ref);
        assert_eq!(container.as_ref(), s_ref.as_bytes());
        match container {
            Container::Ref(_) => {} // Expected
            _ => panic!("Expected Container::Ref from &str"),
        }

        let os_string = OsString::from(VALID_UTF8_STR);
        let container: Container<'_, u8> = Container::from(os_string.clone());
        assert_eq!(container.as_ref(), OsStr::new(VALID_UTF8_STR).as_bytes());
         match container {
            Container::Vec(_) => {} // Expected
            _ => panic!("Expected Container::Vec from OsString"),
        }

        let os_str_ref = OsStr::new(VALID_UTF8_STR);
        let container: Container<'_, u8> = Container::from(os_str_ref);
        assert_eq!(container.as_ref(), os_str_ref.as_bytes());
        match container {
            Container::Ref(_) => {} // Expected
            _ => panic!("Expected Container::Ref from &OsStr"),
        }
    }

    // --- Helper to create containers for testing ---
    fn create_non_locking_containers<'a>(bytes: &'a [u8]) -> Vec<Container<'a, u8>> {
        vec![
            Container::Ref(bytes),
            Container::Vec(bytes.to_vec()),
            Container::Box(bytes.to_vec().into_boxed_slice()),
            Container::Cow(Cow::Borrowed(bytes)),
            Container::Cow(Cow::Owned(bytes.to_vec())),
            Container::Arc(Arc::from(bytes)),
            Container::Rc(Rc::from(bytes)),
        ]
    }

    fn create_locking_containers<'a>(bytes: &'a [u8]) -> Vec<Container<'a, u8>> {
        vec![
            Container::ArcMutexVec(Arc::new(Mutex::new(bytes.to_vec()))),
            Container::ArcRwLockVec(Arc::new(RwLock::new(bytes.to_vec()))),
            Container::RcRefCellVec(Rc::new(RefCell::new(bytes.to_vec()))),
        ]
    }


    #[test]
    fn to_str_etc() {
        // Valid UTF-8
        for container in create_non_locking_containers(VALID_UTF8_BYTES) {
            assert_eq!(container.to_str().unwrap(), VALID_UTF8_STR);
        }
        // Invalid UTF-8
        for container in create_non_locking_containers(INVALID_UTF8_BYTES) {
            assert!(container.to_str().is_err());
        }

        // Valid UTF-8, non-locking
        for container in create_non_locking_containers(VALID_UTF8_BYTES) {
            assert_eq!(container.try_to_str().unwrap(), VALID_UTF8_STR);
        }
        // Invalid UTF-8, non-locking
        for container in create_non_locking_containers(INVALID_UTF8_BYTES) {
            match container.try_to_str() {
                Err(ToStringError::Utf8(_)) => {} // Expected
                _ => panic!("Expected Utf8 error"),
            }
        }

        // Non-locking
        for container in create_non_locking_containers(VALID_UTF8_BYTES) {
            assert_eq!(container.try_to_os_str().unwrap(), OsStr::new(VALID_UTF8_STR));
        }
        for container in create_non_locking_containers(VALID_UTF8_BYTES) {
            assert_eq!(container.to_os_str(), OsStr::new(VALID_UTF8_STR));
        }

        // Locking containers
        for container in create_locking_containers(VALID_UTF8_BYTES) {
            assert_eq!(container.try_to_os_str(), Err(ToStringError::LockRequired));
        }
        for container in create_locking_containers(VALID_UTF8_BYTES) {
            assert_eq!(container.try_to_str(), Err(ToStringError::LockRequired));
        }
    }

    #[test]
    fn get_str_etc() {
        // Valid UTF-8, non-locking
        for container in create_non_locking_containers(VALID_UTF8_BYTES) {
            let (s, guard_option) = container.get_str().unwrap();
            assert_eq!(s, VALID_UTF8_STR);
            assert!(guard_option.is_none());
        }
        // Invalid UTF-8, non-locking
        for container in create_non_locking_containers(INVALID_UTF8_BYTES) {
            match container.get_str() {
                Err(ToStringError::Utf8(_)) => {} // Expected
                _ => panic!("Expected Utf8 error for non-locking invalid"),
            }
        }
        // Valid UTF-8, locking
        for container in create_locking_containers(VALID_UTF8_BYTES) {
            let (s, guard_option) = container.get_str().unwrap();
            assert_eq!(s, VALID_UTF8_STR);
            assert!(guard_option.is_some());
            // Guard is dropped here, lock released
        }
        // Invalid UTF-8, locking
        for container in create_locking_containers(INVALID_UTF8_BYTES) {
             match container.get_str() {
                Err(ToStringError::Utf8(_)) => {} // Expected
                res => panic!("Expected Utf8 error for locking invalid, got {:?}", res),
            }
        }
        // Poisoned lock (Mutex example)
        let data = Arc::new(Mutex::new(VALID_UTF8_BYTES.to_vec()));
        let container_mutex_poisoned: Container<'static, u8> = Container::ArcMutexVec(data.clone());
        // Poison the lock
        let _ = std::thread::spawn(move || {
            let _lock = data.lock().unwrap();
            panic!("intentional panic to poison lock");
        }).join(); // Wait for panic to occur
        // Now try to use it (ArcMutexVec only)
        match container_mutex_poisoned.get_str() {
            Err(ToStringError::LockPoisoned) => {} // Expected
            res => panic!("Expected LockPoisoned error, got {:?}", res),
        }; // Ensure temporary (Result with guard) is dropped
 
        // Valid UTF-8, non-locking
        for container in create_non_locking_containers(VALID_UTF8_BYTES) {
            assert_eq!(container.get_string().unwrap(), VALID_UTF8_STR);
        }
        // Invalid UTF-8, non-locking
        for container in create_non_locking_containers(INVALID_UTF8_BYTES) {
            match container.get_string() {
                Err(ToStringError::FromUtf8(_)) => {} // Expected
                _ => panic!("Expected FromUtf8 error"),
            }
        }
        // Valid UTF-8, locking
        for container in create_locking_containers(VALID_UTF8_BYTES) {
            assert_eq!(container.get_string().unwrap(), VALID_UTF8_STR);
        }
        // Invalid UTF-8, locking
        for container in create_locking_containers(INVALID_UTF8_BYTES) {
             match container.get_string() {
                Err(ToStringError::FromUtf8(_)) => {} // Expected
                _ => panic!("Expected FromUtf8 error for locking invalid"),
            }
        }

        // Valid UTF-8, non-locking
        for container in create_non_locking_containers(VALID_UTF8_BYTES) {
            assert_eq!(container.get_string_lossy().unwrap(), VALID_UTF8_STR);
        }
        // Invalid UTF-8, non-locking (should be replaced)
        for container in create_non_locking_containers(INVALID_UTF8_BYTES) {
            assert_eq!(container.get_string_lossy().unwrap(), "\u{FFFD}");
        }
        // Valid UTF-8, locking
        for container in create_locking_containers(VALID_UTF8_BYTES) {
            assert_eq!(container.get_string_lossy().unwrap(), VALID_UTF8_STR);
        }
        // Invalid UTF-8, locking (should be replaced)
        for container in create_locking_containers(INVALID_UTF8_BYTES) {
             assert_eq!(container.get_string_lossy().unwrap(), "\u{FFFD}");
        }
    }

    #[test]
    fn get_os_str_etc() {
        // Non-locking
        for container in create_non_locking_containers(VALID_UTF8_BYTES) {
            let (s, guard_option) = container.get_os_str().unwrap();
            assert_eq!(s, OsStr::new(VALID_UTF8_STR));
            assert!(guard_option.is_none());
        }
        // Locking
        for container in create_locking_containers(VALID_UTF8_BYTES) {
            let (s, guard_option) = container.get_os_str().unwrap();
            assert_eq!(s, OsStr::new(VALID_UTF8_STR));
            assert!(guard_option.is_some());
        }
        // Poisoned lock (RwLock example)
        let data = Arc::new(RwLock::new(VALID_UTF8_BYTES.to_vec()));
        let container_rwlock_poisoned: Container<'static, u8> = Container::ArcRwLockVec(data.clone());
        // Poison the lock (RwLock write lock)
        let _ = std::thread::spawn(move || {
            let _lock = data.write().unwrap(); // write lock can poison
            panic!("intentional panic to poison rwlock");
        }).join();
        match container_rwlock_poisoned.get_os_str() {
            Err(ToStringError::LockPoisoned) => {} // Expected
            res => panic!("Expected LockPoisoned error for RwLock, got {:?}", res),
        }; // Ensure temporary (Result with guard) is dropped

        // Non-locking
        for container in create_non_locking_containers(VALID_UTF8_BYTES) {
            assert_eq!(container.get_os_string().unwrap(), OsString::from(VALID_UTF8_STR));
        }
        // Locking
        for container in create_locking_containers(VALID_UTF8_BYTES) {
            assert_eq!(container.get_os_string().unwrap(), OsString::from(VALID_UTF8_STR));
        }
    }
}