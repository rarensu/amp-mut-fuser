// This file contains unit tests for the Container enum
// and its trait implementations.
#![allow(clippy::match_wild_err_arm)] // any error fails the test

use crate::container::Container;
use std::sync::Arc;
#[cfg(not(feature = "no-rc"))]
use std::rc::Rc;

#[cfg(test)]
mod t_u8 {
    use super::*;

    #[test]
    fn container_clone_slice() {
        let data_vec: Vec<u8> = vec![1, 2, 3];
        let container1: Container<u8> = Container::from(data_vec.clone()); // This will be Container::Vec
        let container2 = container1.clone();
        assert_eq!(&*container1.unlock().unwrap(), &*container2.unlock().unwrap());
        match (&container1, &container2) {
            (Container::Vec(v1), Container::Vec(v2)) => { // Expect Vec after cloning Vec
                assert_ne!(v1.as_ptr(), v2.as_ptr(), "Cloned Vec<u8> should have different pointers");
            }
            _ => panic!("Expected Vec variants for Container::from(Vec<u8>) clone"),
        }

        let borrowed_slice: &[u8] = &[4,5,6];
        let c_borrowed1 : Container<u8> = Container::Ref(borrowed_slice);
        let c_borrowed2 = c_borrowed1.clone();
        assert_eq!(*c_borrowed1.unlock().unwrap(), *c_borrowed2.unlock().unwrap());
        assert_eq!((*c_borrowed1.unlock().unwrap()).as_ptr(), (*c_borrowed2.unlock().unwrap()).as_ptr());

        let shared_slice_arc: Arc<[u8]> = Arc::new([7,8,9]);
        let c_shared1 : Container<u8> = Container::Arc(shared_slice_arc.clone());
        let c_shared2 = c_shared1.clone();
        if let Container::Arc(s2_arc) = &c_shared2 {
            assert!(Arc::ptr_eq(s2_arc, &shared_slice_arc));
        } else { panic!("Expected shared after clone"); }
    }

    #[test]
    fn container_deref_slice() {
        let data: &[u8] = &[1, 2, 3, 4, 5];
        let container: Container<u8> = Container::from(data);
        let borrowed = container.unlock().unwrap();
        assert_eq!(borrowed.len(), 5); // Test Deref to [u8]
        assert_eq!(borrowed[0], 1);
    }

    use once_cell::sync::Lazy;

    static DATA_BOX: Lazy<Box<[u8]>> = Lazy::new(|| {
        Box::new([4, 5, 6])
    });

    static DATA_VEC: Lazy<Vec<u8>> = Lazy::new(|| {
        vec![7, 8, 9]
    });

    #[test]
    fn borrowed_variants() {
        let data_slice: &'static [u8] = &[1, 2, 3];

        // Container::Ref
        let container_ref = Container::Ref(data_slice);
        assert_eq!(&*container_ref.unlock().unwrap(), data_slice);

        // Container::Cow (borrowed)
        let container_cow_borrowed = Container::Cow(std::borrow::Cow::Borrowed(data_slice));
        assert_eq!(&*container_cow_borrowed.unlock().unwrap(), data_slice);

        // Container::RefBox
        let container_ref_box = Container::RefBox(&DATA_BOX);
        assert_eq!(&*container_ref_box.unlock().unwrap(), DATA_BOX.as_ref());

        // Container::RefVec
        let container_ref_vec = Container::RefVec(&DATA_VEC);
        assert_eq!(&*container_ref_vec.unlock().unwrap(), DATA_VEC.as_slice());

        // Container::CowBox (borrowed)
        let container_cow_box_borrowed = Container::CowBox(std::borrow::Cow::Borrowed(&DATA_BOX));
        assert_eq!(&*container_cow_box_borrowed.unlock().unwrap(), DATA_BOX.as_ref());

        // Container::CowVec (borrowed)
        let container_cow_vec_borrowed = Container::CowVec(std::borrow::Cow::Borrowed(&DATA_VEC));
        assert_eq!(&*container_cow_vec_borrowed.unlock().unwrap(), DATA_VEC.as_slice());
    }

    #[test]
    fn owned_variants() {
        let data_box_orig: Box<[u8]> = Box::new([1, 2, 3]);
        let data_vec_orig: Vec<u8> = vec![4, 5, 6];
        let data_arc_orig: Arc<[u8]> = Arc::new([10, 11, 12]);

        // Container::Box
        let container_box = Container::from(data_box_orig.clone());
        assert_eq!(&*container_box.unlock().unwrap(), data_box_orig.as_ref());

        // Container::Vec
        let container_vec = Container::from(data_vec_orig.clone());
        assert_eq!(&*container_vec.unlock().unwrap(), data_vec_orig.as_slice());

        // Container::Cow (owned from Vec)
        let container_cow_owned_vec = Container::Cow(std::borrow::Cow::Owned(data_vec_orig.clone()));
        assert_eq!(&*container_cow_owned_vec.unlock().unwrap(), data_vec_orig.as_slice());

        // Container::Cow (owned from Box) - Note: Cow doesn't directly take Box<[T]>, so we convert to Vec<T> first for owned Cow
        let container_cow_owned_box_as_vec = Container::Cow(std::borrow::Cow::Owned(data_box_orig.to_vec()));
        assert_eq!(&*container_cow_owned_box_as_vec.unlock().unwrap(), data_box_orig.as_ref());

        #[cfg(not(feature = "no-rc"))]
        {
            // Container::Rc
            let data_rc_orig: Rc<[u8]> = Rc::new([7, 8, 9]);
            let container_rc = Container::from(data_rc_orig.clone());
            assert_eq!(&*container_rc.unlock().unwrap(), data_rc_orig.as_ref());
        }

        // Container::Arc
        let container_arc = Container::from(data_arc_orig.clone());
        assert_eq!(&*container_arc.unlock().unwrap(), data_arc_orig.as_ref());

        #[cfg(not(feature = "no-rc"))]
        {
            // Container::RcBox
            let container_rc_box = Container::RcBox(Rc::new(data_box_orig.clone()));
            assert_eq!(&*container_rc_box.unlock().unwrap(), data_box_orig.as_ref());

            // Container::RcVec
            let container_rc_vec = Container::RcVec(Rc::new(data_vec_orig.clone()));
            assert_eq!(&*container_rc_vec.unlock().unwrap(), data_vec_orig.as_slice());
        }

        // Container::ArcBox
        let container_arc_box = Container::ArcBox(Arc::new(data_box_orig.clone()));
        assert_eq!(&*container_arc_box.unlock().unwrap(), data_box_orig.as_ref());

        // Container::ArcVec
        let container_arc_vec = Container::ArcVec(Arc::new(data_vec_orig.clone()));
        assert_eq!(&*container_arc_vec.unlock().unwrap(), data_vec_orig.as_slice());
    }

    #[cfg(feature = "locking")]
    #[test]
    fn locking_variants_read() {
        let data_box_orig: Box<[u8]> = Box::new([1, 2, 3]);
        let data_vec_orig: Vec<u8> = vec![4, 5, 6];
        
        #[cfg(not(feature = "no-rc"))]
        {
            // Container::RcRefCellBox
            let rc_ref_cell_box = Rc::new(std::cell::RefCell::new(data_box_orig.clone()));
            let container_rc_ref_cell_box: Container<u8> = Container::from(rc_ref_cell_box.clone());
            match container_rc_ref_cell_box.unlock() {
                Ok(guard) => assert_eq!(&*guard, data_box_orig.as_ref()),
                Err(_) => panic!("Expected Ok for RcRefCellBox get_slice"),
            }

            // Container::RcRefCellVec
            let rc_ref_cell_vec = Rc::new(std::cell::RefCell::new(data_vec_orig.clone()));
            let container_rc_ref_cell_vec: Container<u8> = Container::from(rc_ref_cell_vec.clone());
            match container_rc_ref_cell_vec.unlock() {
                Ok(guard) => assert_eq!(&*guard, data_vec_orig.as_slice()),
                Err(_) => panic!("Expected Ok for RcRefCellVec get_slice"),
            }; // this weird semicolon is needed because of lifetimes
        }

        // Container::ArcMutexBox
        let arc_mutex_box = Arc::new(std::sync::Mutex::new(data_box_orig.clone()));
        let container_arc_mutex_box: Container<u8> = Container::from(arc_mutex_box.clone());
        match container_arc_mutex_box.unlock() {
            Ok(guard) => assert_eq!(&*guard, data_box_orig.as_ref()),
            Err(_) => panic!("Expected Ok for ArcMutexBox get_slice"),
        }

        // Container::ArcMutexVec
        let arc_mutex_vec = Arc::new(std::sync::Mutex::new(data_vec_orig.clone()));
        let container_arc_mutex_vec: Container<u8> = Container::from(arc_mutex_vec.clone());
        match container_arc_mutex_vec.unlock() {
            Ok(guard) => assert_eq!(&*guard, data_vec_orig.as_slice()),
            Err(_) => panic!("Expected Ok for ArcMutexVec get_slice"),
        }

        // Container::ArcRwLockBox
        let arc_rw_lock_box = Arc::new(std::sync::RwLock::new(data_box_orig.clone()));
        let container_arc_rw_lock_box: Container<u8> = Container::from(arc_rw_lock_box.clone());
        match container_arc_rw_lock_box.unlock() {
            Ok(guard) => assert_eq!(&*guard, data_box_orig.as_ref()),
            Err(_) => panic!("Expected Ok for ArcRwLockBox get_slice"),
        }

        // Container::ArcRwLockVec
        let arc_rw_lock_vec = Arc::new(std::sync::RwLock::new(data_vec_orig.clone()));
        let container_arc_rw_lock_vec: Container<u8> = Container::from(arc_rw_lock_vec.clone());
        match container_arc_rw_lock_vec.unlock() {
            Ok(guard) => assert_eq!(&*guard, data_vec_orig.as_slice()),
            Err(_) => panic!("Expected Ok for ArcRwLockVec get_slice"),
        };
    }
}