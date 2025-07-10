// This file contains all unit tests for the Container enum
// and its trait implementations.

// Note: When container/mod.rs is set up with `pub use super::core::Container;`
// and `#[cfg(test)] mod tests;`, the tests here can use `use crate::container::Container;`
// or `use super::Container;` if `mod.rs` re-exports `Container` from `core`.
// For now, assuming `crate::container::Container` will be the path once `mod.rs` is set up.
// If not, `super::core::Container` or `crate::container::core::Container` might be needed.

use crate::container::Container; // Assuming Container is re-exported by src/container/mod.rs
use std::ffi::{OsString, OsStr};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
struct MyStruct {
    id: i32,
    data: String,
}

#[test]
fn container_from_borrowed_sized() {
    let data = MyStruct { id: 1, data: "hello".to_string() };
    let container = Container::from(&data);
    match container {
        Container::Borrowed(b) => assert_eq!(b, &data),
        _ => panic!("Expected Borrowed variant"),
    }
    assert_eq!(container.as_ref(), &data);
    assert_eq!(&*container, &data);
}

#[test]
fn container_from_owned_sized() {
    let data = MyStruct { id: 2, data: "world".to_string() };
    let boxed_data = Box::new(data.clone());
    let container = Container::from(boxed_data); // Consumes boxed_data
    match container {
        Container::Owned(ref o) => assert_eq!(o.as_ref(), &data),
        _ => panic!("Expected Owned variant"),
    }
    assert_eq!(container.as_ref(), &data);
}

#[test]
fn container_from_shared_sized() {
    let data = MyStruct { id: 3, data: "share".to_string() };
    let arc_data = Arc::new(data.clone());
    let container = Container::from(arc_data.clone());
    match container {
        Container::Shared(ref s) => {
            assert_eq!(s.as_ref(), &data);
            assert!(Arc::ptr_eq(s, &arc_data));
        }
        _ => panic!("Expected Shared variant"),
    }
    assert_eq!(container.as_ref(), &data);
}

#[test]
fn container_clone_sized() {
    let data = MyStruct { id: 4, data: "clone_me".to_string() };
    let container1 = Container::Owned(Box::new(data.clone()));
    let container2 = container1.clone();
    assert_eq!(container1.as_ref(), container2.as_ref());
    match (container1, container2) {
        (Container::Owned(o1), Container::Owned(o2)) => {
            assert_eq!(o1.id, o2.id);
             // Ensure they are different boxes
            assert_ne!(std::ptr::addr_of!(*o1) as *const _, std::ptr::addr_of!(*o2) as *const _);
        }
        _ => panic!("Expected Owned variants after clone"),
    }

    let borrowed_data = MyStruct { id: 5, data: "borrow_clone".to_string() };
    let c_borrowed1 = Container::Borrowed(&borrowed_data);
    let c_borrowed2 = c_borrowed1.clone();
    assert_eq!(c_borrowed1.as_ref().id, c_borrowed2.as_ref().id);


    let shared_data = MyStruct { id: 6, data: "shared_clone".to_string() };
    let arc_shared = Arc::new(shared_data);
    let c_shared1 = Container::Shared(arc_shared.clone());
    let c_shared2 = c_shared1.clone();
    if let Container::Shared(s2) = &c_shared2 {
        assert!(Arc::ptr_eq(s2, &arc_shared));
    } else {
        panic!("Expected shared")
    }
    assert_eq!(c_shared1.as_ref().id, c_shared2.as_ref().id);
}

// --- Tests for Container<'a, [U]> ---
#[test]
fn container_from_borrowed_slice() {
    let data: &[u8] = &[1, 2, 3];
    let container = Container::from(data);
    match container {
        Container::Borrowed(b) => assert_eq!(b, data),
        _ => panic!("Expected Borrowed variant for slice"),
    }
    assert_eq!(container.as_ref(), data);
}

#[test]
fn container_from_vec() { // Tests From<Vec<U>> for Container<'a, [U]>
    let data_vec: Vec<u8> = vec![4, 5, 6];
    let container: Container<'_, [u8]> = Container::from(data_vec.clone());
    match container {
        Container::Owned(ref o) => assert_eq!(o.as_ref(), data_vec.as_slice()),
        _ => panic!("Expected Owned variant from Vec<u8>"),
    }
    assert_eq!(container.as_ref(), data_vec.as_slice());
}

#[test]
fn container_from_boxed_slice() {
    let data_box: Box<[u8]> = vec![7, 8, 9].into_boxed_slice();
    let container = Container::from(data_box.clone()); // data_box is Box<[u8]>
    match container {
        Container::Owned(ref o) => assert_eq!(o.as_ref(), data_box.as_ref()),
        _ => panic!("Expected Owned variant from Box<[u8]>"),
    }
    assert_eq!(container.as_ref(), data_box.as_ref());
}

#[test]
fn container_from_arc_slice() {
    let data_arc: Arc<[u8]> = Arc::new([10,11,12]);
    let container : Container<'_, [u8]> = Container::from(data_arc.clone());
     match container {
        Container::Shared(ref s) => {
            assert_eq!(s.as_ref(), data_arc.as_ref());
            assert!(Arc::ptr_eq(s, &data_arc));
        }
        _ => panic!("Expected Shared variant from Arc<[u8]>"),
    }
    assert_eq!(container.as_ref(), data_arc.as_ref());
}

#[test]
fn container_clone_slice() {
    let data_vec: Vec<u8> = vec![1, 2, 3];
    let container1: Container<'_, [u8]> = Container::from(data_vec.clone()); // Owned
    let container2 = container1.clone();
    assert_eq!(container1.as_ref(), container2.as_ref());
    match (&container1, &container2) {
        (Container::Owned(o1), Container::Owned(o2)) => {
            assert_ne!(o1.as_ptr(), o2.as_ptr(), "Cloned Box<[u8]> should have different pointers");
        }
        _ => panic!("Expected Owned variants for slice clone"),
    }

    let borrowed_slice: &[u8] = &[4,5,6];
    let c_borrowed1 : Container<'_, [u8]> = Container::Borrowed(borrowed_slice);
    let c_borrowed2 = c_borrowed1.clone();
    assert_eq!(c_borrowed1.as_ref(), c_borrowed2.as_ref());
    assert_eq!(c_borrowed1.as_ref().as_ptr(), c_borrowed2.as_ref().as_ptr());


    let shared_slice_arc: Arc<[u8]> = Arc::new([7,8,9]);
    let c_shared1 : Container<'_, [u8]> = Container::Shared(shared_slice_arc.clone());
    let c_shared2 = c_shared1.clone();
    if let Container::Shared(s2_arc) = &c_shared2 {
        assert!(Arc::ptr_eq(s2_arc, &shared_slice_arc));
    } else { panic!("Expected shared after clone"); }
}

// --- Tests for Container<'a, str> ---
#[test]
fn container_from_borrowed_str() {
    let data: &str = "hello static";
    let container: Container<'_, str> = Container::from(data); // Explicit type for clarity and to fix E0282
    match container {
        Container::Borrowed(b) => assert_eq!(b, data),
        _ => panic!("Expected Borrowed variant for &str"),
    }
    assert_eq!(container.as_ref(), data);
}

#[test]
fn container_from_string_to_osstr_container() { // From<String> for Container<'a, OsStr>
    let data_string: String = "hello owned string".to_string();
    // Note: This From<String> is for Container<'a, OsStr>, not Container<'a, str>
    // To get Container<'a, str>::Owned from String, you'd do Container::Owned(string.into_boxed_str())
    let container: Container<'_, OsStr> = Container::from(data_string.clone());
    match container {
        Container::Owned(ref o) => assert_eq!(o.as_ref(), OsStr::new(&data_string)), // Compare AsRef
        _ => panic!("Expected Owned variant from String for OsStr container"),
    }
}

#[test]
fn container_owned_from_boxed_str() {
    let data_string = "boxed str content".to_string();
    let boxed_s: Box<str> = data_string.into_boxed_str();
    let container: Container<'_, str> = Container::Owned(boxed_s.clone());
     match container {
        Container::Owned(ref o) => assert_eq!(o.as_ref(), boxed_s.as_ref()),
        _ => panic!("Expected Owned variant from Box<str>"),
    }
}

#[test]
fn container_shared_from_arc_str() {
    let data_string = "arc str content".to_string();
    let arc_s: Arc<str> = Arc::from(data_string);
    let container: Container<'_, str> = Container::Shared(arc_s.clone());
     match container {
        Container::Shared(ref s_val) => {
            assert_eq!(s_val.as_ref(), arc_s.as_ref());
            assert!(Arc::ptr_eq(s_val, &arc_s));
        }
        _ => panic!("Expected Shared variant from Arc<str>"),
    }
}

#[test]
fn container_clone_str() {
    let data_string = "clone this str".to_string();
    let container1: Container<'_, str> = Container::Owned(data_string.into_boxed_str());
    let container2 = container1.clone();
    assert_eq!(container1.as_ref(), container2.as_ref());
     match (&container1, &container2) {
        (Container::Owned(o1), Container::Owned(o2)) => {
             assert_ne!(o1.as_ptr(), o2.as_ptr(), "Cloned Box<str> should have different pointers");
        }
        _ => panic!("Expected Owned variants for str clone"),
    }
}


// --- Tests for Container<'a, OsStr> ---
#[test]
fn container_from_borrowed_osstr_from_str_literal() { // From<&'a str> for Container<'a, OsStr>
    let data: &str = "hello os_str";
    let container: Container<'_, OsStr> = Container::from(data);
    match container {
        Container::Borrowed(b) => assert_eq!(b, OsStr::new(data)),
        _ => panic!("Expected Borrowed variant for OsStr from &str"),
    }
    assert_eq!(container.as_ref(), OsStr::new(data));
}

#[test]
fn container_from_borrowed_osstr() {
    let data_osstr: &OsStr = OsStr::new("real osstr");
    let container = Container::from(data_osstr); // Container<'a, OsStr>
    match container {
        Container::Borrowed(b) => assert_eq!(b, data_osstr),
        _ => panic!("Expected Borrowed variant for &OsStr"),
    }
    assert_eq!(container.as_ref(), data_osstr);
}

#[test]
fn container_from_osstring() { // From<OsString> for Container<'a, OsStr>
    let data_osstring = OsString::from("hello os_string");
    let container: Container<'_, OsStr> = Container::from(data_osstring.clone());
    match container {
        Container::Owned(ref o) => assert_eq!(o.as_ref(), data_osstring.as_os_str()),
        _ => panic!("Expected Owned variant from OsString"),
    }
    assert_eq!(container.as_ref(), data_osstring.as_os_str());
}

#[test]
fn container_from_boxed_osstr() {
    let data_osstring = OsString::from("boxed os_str content");
    let boxed_osstr: Box<OsStr> = data_osstring.into_boxed_os_str();
    let container: Container<'_, OsStr> = Container::from(boxed_osstr.clone());
     match container {
        Container::Owned(ref o) => assert_eq!(o.as_ref(), boxed_osstr.as_ref()),
        _ => panic!("Expected Owned variant from Box<OsStr>"),
    }
}

#[test]
fn container_from_arc_osstr() {
    let data_osstring = OsString::from("arc os_str content");
    let arc_osstr: Arc<OsStr> = Arc::from(data_osstring.into_boxed_os_str());
    let container: Container<'_, OsStr> = Container::from(arc_osstr.clone());
     match container {
        Container::Shared(ref s_val) => {
            assert_eq!(s_val.as_ref(), arc_osstr.as_ref());
            assert!(Arc::ptr_eq(s_val, &arc_osstr));
        }
        _ => panic!("Expected Shared variant from Arc<OsStr>"),
    }
}

#[test]
fn container_clone_osstr() {
    let data_osstring = OsString::from("clone this os_str");
    let container1: Container<'_, OsStr> = Container::from(data_osstring.clone()); // Owned
    let container2 = container1.clone();
    assert_eq!(container1.as_ref(), container2.as_ref());
    match (&container1, &container2) {
        (Container::Owned(o1), Container::Owned(o2)) => {
            // Ensure they are different boxes by comparing their addresses
            assert_ne!(std::ptr::addr_of!(**o1) as *const _, std::ptr::addr_of!(**o2) as *const _, "Cloned Box<OsStr> should point to different memory locations");
        }
        _ => panic!("Expected Owned variants for OsStr clone"),
    }
}

#[test]
fn container_deref_slice() {
    let data: &[u8] = &[1, 2, 3, 4, 5];
    let container: Container<'_, [u8]> = Container::from(data);
    assert_eq!(container.len(), 5); // Test Deref to [u8]
    assert_eq!(container[0], 1);
}

#[test]
fn container_deref_str() {
    let data: &str = "hello";
    let container: Container<'_, str> = Container::from(data);
    assert_eq!(container.len(), 5); // Test Deref to str
    assert!(container.starts_with("h"));
}

#[test]
fn container_deref_osstr() {
    let data: &OsStr = OsStr::new("os_hello");
    let container: Container<'_, OsStr> = Container::from(data);
    assert_eq!(container.len(), 8); // Test Deref to OsStr
    assert_eq!(container.as_ref(), OsStr::new("os_hello")); // Compare AsRef
}
