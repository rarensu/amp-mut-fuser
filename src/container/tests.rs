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
fn container_from_borrowed_slice() {
    let data: &[u8] = &[1, 2, 3];
    let container = Container::from(data);
    match container {
        Container::Ref(b) => assert_eq!(b, data),
        _ => panic!("Expected Borrowed variant for slice"),
    }
    assert_eq!(container.as_ref(), data);
}

#[test]
fn container_from_vec() {
    let data_vec: Vec<u8> = vec![4, 5, 6];
    let container: Container<'_, u8> = Container::from(data_vec.clone());
    assert_eq!(container.as_ref(), data_vec.as_slice());
}

#[test]
fn container_from_boxed_slice() {
    let data_box: Box<[u8]> = vec![7, 8, 9].into_boxed_slice();
    let container = Container::from(data_box.clone()); // data_box is Box<[u8]>
    match container {
        Container::Box(ref o) => assert_eq!(o.as_ref(), data_box.as_ref()),
        _ => panic!("Expected Box variant from Box<[u8]>"),
    }
    assert_eq!(container.as_ref(), data_box.as_ref());
}

#[test]
fn container_from_arc_slice() {
    let data_arc: Arc<[u8]> = Arc::new([10,11,12]);
    let container : Container<'_, u8> = Container::from(data_arc.clone());
     match container {
        Container::Arc(ref s) => {
            assert_eq!(s.as_ref(), data_arc.as_ref());
            assert!(Arc::ptr_eq(s, &data_arc));
        }
        _ => panic!("Expected Arc variant from Arc<[u8]>"),
    }
    assert_eq!(container.as_ref(), data_arc.as_ref());
}

#[test]
fn container_clone_slice() {
    let data_vec: Vec<u8> = vec![1, 2, 3];
    let container1: Container<'_, u8> = Container::from(data_vec.clone()); // Box
    let container2 = container1.clone();
    assert_eq!(container1.as_ref(), container2.as_ref());
    match (&container1, &container2) {
        (Container::Box(o1), Container::Box(o2)) => {
            assert_ne!(o1.as_ptr(), o2.as_ptr(), "Cloned Box<[u8]> should have different pointers");
        }
        _ => panic!("Expected Box variants for slice clone"),
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