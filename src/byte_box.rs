use std::sync::Arc;

/// `ByteBox` is an enum that provides a flexible way to return byte slices (`&[u8]`)
/// from `Filesystem` trait methods like `read` and `readlink`. It allows filesystem
/// implementations to return data with different ownership models, optimizing for
/// performance by enabling zero-copy reads where possible.
///
/// - `Borrowed`: For data that already exists in memory and can be borrowed (e.g., static data,
///   mmap'ed file contents). This is the most performant option as it avoids copying.
/// - `Owned`: For newly allocated data that is owned by the `ByteBox` (e.g., data read into a
///   `Vec<u8>` and then converted).
/// - `Shared`: For data that is shared across multiple parts of the filesystem or needs to
///   outlive the current request scope (e.g., cached data wrapped in an `Arc`).
#[derive(Debug)]
pub enum ByteBox<'a> {
    /// A borrowed slice of bytes. This variant should be used when the data
    /// can be safely borrowed for the lifetime `'a`.
    Borrowed(&'a [u8]),
    /// An owned, heap-allocated slice of bytes (`Box<[u8]>`).
    Owned(Box<[u8]>),
    /// A shared, atomically reference-counted slice of bytes (`Arc<[u8]>`).
    Shared(Arc<[u8]>),
}

impl<'a> From<&'a [u8]> for ByteBox<'a> {
    fn from(slice: &'a [u8]) -> Self {
        ByteBox::Borrowed(slice)
    }
}

// --- OsBox ---

use std::ffi::{OsStr, OsString};

/// `OsBox` provides a flexible way to handle OS-native string data (`OsStr`)
/// with different ownership models, similar to `ByteBox` for byte slices.
/// This is particularly useful for `Filesystem` trait methods that return path
/// components or symbolic link targets.
///
/// - `Borrowed`: For `&OsStr` that can be borrowed (e.g., static `OsStr`s).
/// - `Owned`: For an owned, heap-allocated `OsStr` (i.e., `Box<OsStr>`).
/// - `Shared`: For a shared, atomically reference-counted `OsStr` (i.e., `Arc<OsStr>`).
#[derive(Debug)]
pub enum OsBox<'a> {
    /// A borrowed `OsStr`.
    Borrowed(&'a OsStr),
    /// An owned, heap-allocated `OsStr`. This is `Clone` as `Box<OsStr>` is `Clone`.
    Owned(Box<OsStr>),
    /// A shared, atomically reference-counted `OsStr`.
    Shared(Arc<OsStr>),
}

impl<'a> From<&'a OsStr> for OsBox<'a> {
    fn from(s: &'a OsStr) -> Self {
        OsBox::Borrowed(s)
    }
}

impl<'a> From<OsString> for OsBox<'a> {
    fn from(s: OsString) -> Self {
        OsBox::Owned(s.into_boxed_os_str())
    }
}

impl<'a> From<Box<OsStr>> for OsBox<'a> {
    fn from(b: Box<OsStr>) -> Self {
        OsBox::Owned(b)
    }
}

impl<'a> From<Arc<OsStr>> for OsBox<'a> {
    fn from(a: Arc<OsStr>) -> Self {
        OsBox::Shared(a)
    }
}

impl<'a> AsRef<OsStr> for OsBox<'a> {
    fn as_ref(&self) -> &OsStr {
        match self {
            OsBox::Borrowed(s) => s,
            OsBox::Owned(b) => b.as_ref(),
            OsBox::Shared(a) => a.as_ref(),
        }
    }
}

impl<'a> Deref for OsBox<'a> {
    type Target = OsStr;

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl<'a> Clone for OsBox<'a> {
    fn clone(&self) -> Self {
        match self {
            OsBox::Borrowed(s) => OsBox::Borrowed(s),
            OsBox::Owned(b) => OsBox::Owned(b.clone()), // Box<OsStr> is Clone
            OsBox::Shared(a) => OsBox::Shared(a.clone()), // Arc<OsStr> is Clone (bumps ref count)
        }
    }
}

#[cfg(test)]
mod tests_os_box {
    use super::*;
    use std::ffi::{OsStr, OsString};
    use std::sync::Arc;

    #[test]
    fn os_box_borrowed() {
        let data: &OsStr = OsStr::new("test_borrowed");
        let os_box = OsBox::from(data);
        match os_box {
            OsBox::Borrowed(s) => assert_eq!(s, data),
            _ => panic!("Expected OsBox::Borrowed"),
        }
        assert_eq!(os_box.as_ref(), data);
        assert_eq!(&*os_box, data);
    }

    #[test]
    fn os_box_owned_from_os_string() {
        let data_os_string = OsString::from("test_owned_osstring");
        let os_box = OsBox::from(data_os_string.clone()); // clone because From<OsString> consumes
        match os_box {
            OsBox::Owned(ref b) => assert_eq!(b.as_ref(), data_os_string.as_os_str()),
            _ => panic!("Expected OsBox::Owned from OsString"),
        }
        assert_eq!(os_box.as_ref(), data_os_string.as_os_str());
    }

    #[test]
    fn os_box_owned_from_box_os_str() {
        let data_box: Box<OsStr> = OsString::from("test_owned_box").into_boxed_os_str();
        let os_box = OsBox::from(data_box.clone()); // clone because From<Box<OsStr>> consumes
        match os_box {
            OsBox::Owned(ref b) => assert_eq!(b.as_ref(), data_box.as_ref()),
            _ => panic!("Expected OsBox::Owned from Box<OsStr>"),
        }
        assert_eq!(os_box.as_ref(), data_box.as_ref());
    }

    #[test]
    fn os_box_shared_from_arc_os_str() {
        let data_arc: Arc<OsStr> = Arc::from(OsString::from("test_shared_arc").into_boxed_os_str());
        let os_box = OsBox::from(data_arc.clone());
        match os_box {
            OsBox::Shared(ref a) => assert_eq!(a.as_ref(), data_arc.as_ref()),
            _ => panic!("Expected OsBox::Shared from Arc<OsStr>"),
        }
        assert_eq!(os_box.as_ref(), data_arc.as_ref());
        assert!(Arc::ptr_eq(
            match os_box { OsBox::Shared(ref a) => a, _ => panic!() },
            &data_arc
        ));
    }

    #[test]
    fn os_box_clone() {
        let static_os_str: &'static OsStr = OsStr::new("static_val");

        let b1 = OsBox::Borrowed(static_os_str);
        let b2 = b1.clone();
        assert_eq!(b1.as_ref(), b2.as_ref());
        if let OsBox::Borrowed(s) = b2 {
            assert_eq!(s, static_os_str);
        } else { panic!(); }

        let owned_os_string = OsString::from("owned_val");
        let o1 = OsBox::from(owned_os_string.clone());
        let o2 = o1.clone();
        assert_eq!(o1.as_ref(), o2.as_ref());
        if let OsBox::Owned(ref b_val) = o2 { // o2 is OsBox::Owned(Box<OsStr>)
            assert_eq!(b_val.as_ref(), owned_os_string.as_os_str());
            // Ensure it's a new Box, not just a reference copy of the Box itself
            if let OsBox::Owned(ref orig_b_val) = o1 {
                 assert_ne!(std::ptr::addr_of!(**orig_b_val) as *const u8, std::ptr::addr_of!(**b_val) as *const u8);
            }
        } else { panic!(); }


        let shared_arc: Arc<OsStr> = Arc::from(OsString::from("shared_val").into_boxed_os_str());
        let s1 = OsBox::from(shared_arc.clone());
        let s2 = s1.clone();
        assert_eq!(s1.as_ref(), s2.as_ref());
        if let OsBox::Shared(ref arc_val) = s2 {
            assert!(Arc::ptr_eq(arc_val, &shared_arc));
        } else { panic!(); }
    }
}

// --- DirEntryBox ---

/// `DirEntryBox` is an enum analogous to `ByteBox`, but for returning slices of directory
/// entries (e.g., `DirEntry` from the `readdir` method). It allows for different
/// ownership models to optimize performance.
///
/// The type parameter `DE` is typically `DirEntry`.
///
/// - `Borrowed`: For a slice of directory entries that can be borrowed.
/// - `Owned`: For a newly allocated, owned list of directory entries.
/// - `Shared`: For a shared, reference-counted list of directory entries.
#[derive(Debug)]
pub enum DirEntryBox<'a, DE> {
    /// A borrowed slice of directory entries.
    Borrowed(&'a [DE]),
    /// An owned, heap-allocated slice of directory entries (`Box<[DE]>`).
    Owned(Box<[DE]>),
    /// A shared, atomically reference-counted slice of directory entries (`Arc<[DE]>`).
    Shared(Arc<[DE]>),
}

impl<'a, DE> From<&'a [DE]> for DirEntryBox<'a, DE> where DE: 'a {
    fn from(slice: &'a [DE]) -> Self {
        DirEntryBox::Borrowed(slice)
    }
}

impl<'a, DE> From<Vec<DE>> for DirEntryBox<'a, DE> {
    fn from(vec: Vec<DE>) -> Self {
        DirEntryBox::Owned(vec.into_boxed_slice())
    }
}

impl<'a, DE> From<Box<[DE]>> for DirEntryBox<'a, DE> {
    fn from(boxed_slice: Box<[DE]>) -> Self {
        DirEntryBox::Owned(boxed_slice)
    }
}

impl<'a, DE> From<Arc<[DE]>> for DirEntryBox<'a, DE> {
    fn from(arc_slice: Arc<[DE]>) -> Self {
        DirEntryBox::Shared(arc_slice)
    }
}

impl<'a, DE> AsRef<[DE]> for DirEntryBox<'a, DE> {
    fn as_ref(&self) -> &[DE] {
        match self {
            DirEntryBox::Borrowed(slice) => slice,
            DirEntryBox::Owned(boxed_slice) => boxed_slice,
            DirEntryBox::Shared(arc_slice) => arc_slice,
        }
    }
}

impl<'a, DE> Deref for DirEntryBox<'a, DE> {
    type Target = [DE];

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}


// --- DirEntryPlusBox ---
// This is identical in structure to DirEntryBox, but typically DE will be (DirEntry, Entry)
// We could use a type alias, but a distinct type might be clearer if specific methods were added later.
// For now, it's a direct copy-paste with a different name.
// pub type DirEntryPlusBox<'a, DEP> = DirEntryBox<'a, DEP>;
// Using a new type for clarity, even if structurally identical for now.

/// `DirEntryPlusBox` is similar to `DirEntryBox` but designed for methods like
/// `readdirplus` that return directory entries along with their attributes
/// (e.g., a slice of `(DirEntry, Entry)` tuples).
///
/// The type parameter `DEP` is typically `(DirEntry, Entry)`.
///
/// - `Borrowed`: For a slice of extended directory entries that can be borrowed.
/// - `Owned`: For a newly allocated, owned list of extended directory entries.
/// - `Shared`: For a shared, reference-counted list of extended directory entries.
#[derive(Debug)]
pub enum DirEntryPlusBox<'a, DEP> {
    /// A borrowed slice of extended directory entries.
    Borrowed(&'a [DEP]),
    /// An owned, heap-allocated slice of extended directory entries (`Box<[DEP]>`).
    Owned(Box<[DEP]>),
    /// A shared, atomically reference-counted slice of extended directory entries (`Arc<[DEP]>`).
    Shared(Arc<[DEP]>),
}

impl<'a, DEP> From<&'a [DEP]> for DirEntryPlusBox<'a, DEP> where DEP: 'a {
    fn from(slice: &'a [DEP]) -> Self {
        DirEntryPlusBox::Borrowed(slice)
    }
}

impl<'a, DEP> From<Vec<DEP>> for DirEntryPlusBox<'a, DEP> {
    fn from(vec: Vec<DEP>) -> Self {
        DirEntryPlusBox::Owned(vec.into_boxed_slice())
    }
}

impl<'a, DEP> From<Box<[DEP]>> for DirEntryPlusBox<'a, DEP> {
    fn from(boxed_slice: Box<[DEP]>) -> Self {
        DirEntryPlusBox::Owned(boxed_slice)
    }
}

impl<'a, DEP> From<Arc<[DEP]>> for DirEntryPlusBox<'a, DEP> {
    fn from(arc_slice: Arc<[DEP]>) -> Self {
        DirEntryPlusBox::Shared(arc_slice)
    }
}

impl<'a, DEP> AsRef<[DEP]> for DirEntryPlusBox<'a, DEP> {
    fn as_ref(&self) -> &[DEP] {
        match self {
            DirEntryPlusBox::Borrowed(slice) => slice,
            DirEntryPlusBox::Owned(boxed_slice) => boxed_slice,
            DirEntryPlusBox::Shared(arc_slice) => arc_slice,
        }
    }
}

impl<'a, DEP> Deref for DirEntryPlusBox<'a, DEP> {
    type Target = [DEP];

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}


#[cfg(test)]
mod tests_dir_entry {
    use super::*;
    use crate::{DirEntry, Entry, FileAttr, FileType}; // Assuming these are available from crate root
    use std::ffi::OsString;
    use std::sync::Arc;
    use std::time::{Duration, UNIX_EPOCH};

    // Mock DirEntry and Entry for testing purposes if they are not easily constructible
    // For now, assume they can be created. Need to ensure crate::DirEntry etc. are accessible.
    // If not, we might need to define simplified versions here or adjust imports.
    // Let's use dummy structures for DirEntry and Entry if actual ones are complex.

    fn dummy_file_attr(ino: u64) -> FileAttr {
        FileAttr {
            ino,
            size: 0,
            blocks: 0,
            atime: UNIX_EPOCH,
            mtime: UNIX_EPOCH,
            ctime: UNIX_EPOCH,
            crtime: UNIX_EPOCH,
            kind: FileType::RegularFile,
            perm: 0,
            nlink: 0,
            uid: 0,
            gid: 0,
            rdev: 0,
            blksize: 0,
            flags: 0,
        }
    }

    fn dummy_dir_entry(ino: u64, name: &str) -> DirEntry {
        DirEntry {
            ino,
            offset: 0,
            kind: FileType::RegularFile,
            name: OsString::from(name),
        }
    }

    fn dummy_entry_data(ino: u64) -> Entry {
        Entry {
            attr: dummy_file_attr(ino),
            ttl: Duration::from_secs(1),
            generation: 0,
        }
    }


    #[test]
    fn dir_entry_box_borrowed() {
        let entries = [dummy_dir_entry(1, "file1")];
        let dir_entry_box = DirEntryBox::from(&entries[..]);
        match dir_entry_box {
            DirEntryBox::Borrowed(b) => assert_eq!(b.len(), 1),
            _ => panic!("Expected DirEntryBox::Borrowed"),
        }
        assert_eq!(dir_entry_box.as_ref().len(), 1);
    }

    #[test]
    fn dir_entry_box_owned_from_vec() {
        let entries_vec = vec![dummy_dir_entry(2, "file2")];
        let dir_entry_box = DirEntryBox::from(entries_vec.clone());
        match dir_entry_box {
            DirEntryBox::Owned(ref b) => assert_eq!(b.len(), 1),
            _ => panic!("Expected DirEntryBox::Owned"),
        }
        assert_eq!(dir_entry_box.as_ref().len(), 1);
    }

    #[test]
    fn dir_entry_plus_box_shared() {
        let entries_plus_vec: Vec<(DirEntry, Entry)> = vec![(dummy_dir_entry(3, "file3"), dummy_entry_data(3))];
        let arc_entries: Arc<[(DirEntry, Entry)]> = Arc::from(entries_plus_vec);
        let dir_entry_plus_box = DirEntryPlusBox::from(arc_entries.clone());
        match dir_entry_plus_box {
            DirEntryPlusBox::Shared(ref a) => assert_eq!(a.len(), 1),
            _ => panic!("Expected DirEntryPlusBox::Shared"),
        }
        assert_eq!(dir_entry_plus_box.as_ref().len(), 1);
        assert!(Arc::ptr_eq(
            match dir_entry_plus_box { DirEntryPlusBox::Shared(ref a) => a, _ => panic!() },
            &arc_entries
        ));
    }
}

impl<'a> From<Vec<u8>> for ByteBox<'a> {
    fn from(vec: Vec<u8>) -> Self {
        ByteBox::Owned(vec.into_boxed_slice())
    }
}

impl<'a> From<Box<[u8]>> for ByteBox<'a> {
    fn from(boxed_slice: Box<[u8]>) -> Self {
        ByteBox::Owned(boxed_slice)
    }
}

impl<'a> From<Arc<[u8]>> for ByteBox<'a> {
    fn from(arc_slice: Arc<[u8]>) -> Self {
        ByteBox::Shared(arc_slice)
    }
}

// It's useful to be able to get a reference to the underlying bytes easily.
impl<'a> AsRef<[u8]> for ByteBox<'a> {
    fn as_ref(&self) -> &[u8] {
        match self {
            ByteBox::Borrowed(slice) => slice,
            ByteBox::Owned(boxed_slice) => boxed_slice,
            ByteBox::Shared(arc_slice) => arc_slice,
        }
    }
}

// Adding Deref might also be convenient for some use cases,
// though AsRef is often more explicit and preferred for generic code.
use std::ops::Deref;
impl<'a> Deref for ByteBox<'a> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn byte_box_borrowed() {
        let data: &[u8] = &[1, 2, 3];
        let byte_box = ByteBox::from(data);
        match byte_box {
            ByteBox::Borrowed(b) => assert_eq!(b, data),
            _ => panic!("Expected ByteBox::Borrowed"),
        }
        assert_eq!(byte_box.as_ref(), data);
        assert_eq!(&*byte_box, data);
    }

    #[test]
    fn byte_box_owned_from_vec() {
        let data_vec: Vec<u8> = vec![4, 5, 6];
        let byte_box = ByteBox::from(data_vec.clone()); // clone because from(Vec) consumes
        match byte_box {
            ByteBox::Owned(ref b) => assert_eq!(b.as_ref(), data_vec.as_slice()),
            _ => panic!("Expected ByteBox::Owned"),
        }
        assert_eq!(byte_box.as_ref(), data_vec.as_slice());
    }

    #[test]
    fn byte_box_owned_from_box() {
        let data_box: Box<[u8]> = vec![7, 8, 9].into_boxed_slice();
        let byte_box = ByteBox::from(data_box.clone()); // clone because from(Box) consumes
        match byte_box {
            ByteBox::Owned(ref b) => assert_eq!(b.as_ref(), data_box.as_ref()),
            _ => panic!("Expected ByteBox::Owned"),
        }
        assert_eq!(byte_box.as_ref(), data_box.as_ref());
    }

    #[test]
    fn byte_box_shared_from_arc() {
        let data_arc: Arc<[u8]> = Arc::new([10, 11, 12]);
        let byte_box = ByteBox::from(data_arc.clone()); // clone because from(Arc) consumes Arc, not the data
        match byte_box {
            ByteBox::Shared(ref a) => assert_eq!(a.as_ref(), data_arc.as_ref()),
            _ => panic!("Expected ByteBox::Shared"),
        }
        assert_eq!(byte_box.as_ref(), data_arc.as_ref());
        assert!(Arc::ptr_eq(
            match byte_box { ByteBox::Shared(ref a) => a, _ => panic!() },
            &data_arc
        ));
    }

    #[test]
    fn byte_box_as_ref() {
        let data_static: &'static [u8] = b"static";
        let bb_borrowed = ByteBox::from(data_static);
        assert_eq!(bb_borrowed.as_ref(), b"static");

        let data_vec = vec![1,2,3];
        let bb_owned_vec = ByteBox::from(data_vec.clone());
        assert_eq!(bb_owned_vec.as_ref(), data_vec.as_slice());

        let data_box: Box<[u8]> = vec![4,5,6u8].into_boxed_slice();
        let bb_owned_box = ByteBox::from(data_box.clone());
        assert_eq!(bb_owned_box.as_ref(), data_box.as_ref());

        let data_arc: Arc<[u8]> = Arc::from(vec![7,8,9u8]);
        let bb_shared_arc = ByteBox::from(data_arc.clone());
        assert_eq!(bb_shared_arc.as_ref(), data_arc.as_ref());
    }

    #[test]
    fn byte_box_deref() {
        let data_static: &'static [u8] = b"static_deref";
        let bb_borrowed: ByteBox<'_> = ByteBox::from(data_static);
        assert_eq!(&*bb_borrowed, b"static_deref");
        assert_eq!(bb_borrowed.len(), b"static_deref".len());
    }
}
