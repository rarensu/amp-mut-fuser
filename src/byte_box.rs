use std::ffi::{OsStr, OsString};
use std::ops::Deref;
use std::sync::Arc;

// --- ByteBox ---
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

impl From<Vec<u8>> for ByteBox<'_> {
    fn from(vec: Vec<u8>) -> Self {
        ByteBox::Owned(vec.into_boxed_slice())
    }
}

impl From<Box<[u8]>> for ByteBox<'_> {
    fn from(boxed_slice: Box<[u8]>) -> Self {
        ByteBox::Owned(boxed_slice)
    }
}

impl From<Arc<[u8]>> for ByteBox<'_> {
    fn from(arc_slice: Arc<[u8]>) -> Self {
        ByteBox::Shared(arc_slice)
    }
}

impl AsRef<[u8]> for ByteBox<'_> {
    fn as_ref(&self) -> &[u8] {
        match self {
            ByteBox::Borrowed(slice) => slice,
            ByteBox::Owned(boxed_slice) => boxed_slice,
            ByteBox::Shared(arc_slice) => arc_slice,
        }
    }
}

impl Deref for ByteBox<'_> {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

// --- OsBox ---

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
    /// A borrowed `&'a OsStr`.
    Borrowed(&'a OsStr),
    /// An owned `Box<OsStr>`.
    Owned(Box<OsStr>),
    /// A shared `Arc<OsStr>`.
    Shared(Arc<OsStr>),
}

impl<'a> From<&'a OsStr> for OsBox<'a> {
    fn from(s: &'a OsStr) -> Self { OsBox::Borrowed(s) }
}
impl From<OsString> for OsBox<'_> {
    fn from(s: OsString) -> Self { OsBox::Owned(s.into_boxed_os_str()) }
}
impl From<Box<OsStr>> for OsBox<'_> {
    fn from(b: Box<OsStr>) -> Self { OsBox::Owned(b) }
}
impl From<Arc<OsStr>> for OsBox<'_> {
    fn from(a: Arc<OsStr>) -> Self { OsBox::Shared(a) }
}
impl AsRef<OsStr> for OsBox<'_> {
    fn as_ref(&self) -> &OsStr {
        match self {
            OsBox::Borrowed(s) => s,
            OsBox::Owned(b) => b.as_ref(),
            OsBox::Shared(a) => a.as_ref(),
        }
    }
}
impl Deref for OsBox<'_> {
    type Target = OsStr;
    fn deref(&self) -> &Self::Target { self.as_ref() }
}
impl Clone for OsBox<'_> {
    fn clone(&self) -> Self {
        match self {
            OsBox::Borrowed(s) => OsBox::Borrowed(s),
            OsBox::Owned(b) => OsBox::Owned(b.clone()),
            OsBox::Shared(a) => OsBox::Shared(a.clone()),
        }
    }
}

// --- DirEntryData related Box types ---

// DirEntryData itself is defined in src/reply.rs
// For DirEntryPlusData, we need fuser::Entry for attributes
use crate::reply::DirEntryData;
use crate::Entry as FuserEntry;

/// `DirEntryContainer` allows a single directory entry (`DirEntryData`) to be returned
/// with flexible ownership, enabling borrowing of static/cached entries or returning owned ones.
///
/// The `'entry` lifetime is for the `DirEntryData` if it's borrowed.
/// The `'name` lifetime is for the name within the `DirEntryData` if that name is borrowed.
#[derive(Debug, Clone)]
pub enum DirEntryContainer<'entry, 'name> {
    // 'entry is the lifetime of a borrowed entry: DirEntry. 
    // 'name is the lifetime of a borrowed name: OsString.
    /// A borrowed `DirEntryData` struct. The lifetime `'entry` applies to this borrow.
    Borrowed(&'entry DirEntryData<'name>),
    /// An owned `DirEntryData` struct.
    Owned(DirEntryData<'name>),
    /// A shared, atomically reference-counted `DirEntryData` struct.
    Shared(Arc<DirEntryData<'name>>),
}

impl<'name> AsRef<DirEntryData<'name>> for DirEntryContainer<'_, 'name> {
    fn as_ref(&self) -> &DirEntryData<'name> {
        match self {
            DirEntryContainer::Borrowed(entry_data) => entry_data,
            DirEntryContainer::Owned(entry_data) => entry_data,
            DirEntryContainer::Shared(arc_entry_data) => arc_entry_data.as_ref(),
        }
    }
}

impl<'name> Deref for DirEntryContainer<'_, 'name> {
    type Target = DirEntryData<'name>;
    fn deref(&self) -> &Self::Target { self.as_ref() }
}

impl<'entry, 'name> From<&'entry DirEntryData<'name>> for DirEntryContainer<'entry, 'name> {
    fn from(entry_data_ref: &'entry DirEntryData<'name>) -> Self {
        DirEntryContainer::Borrowed(entry_data_ref)
    }
}

impl<'name> From<DirEntryData<'name>> for DirEntryContainer<'_, 'name> {
    fn from(entry_data: DirEntryData<'name>) -> Self {
        DirEntryContainer::Owned(entry_data)
    }
}

impl<'name> From<Arc<DirEntryData<'name>>> for DirEntryContainer<'_, 'name> {
    fn from(arc_entry_data: Arc<DirEntryData<'name>>) -> Self {
        DirEntryContainer::Shared(arc_entry_data)
    }
}

/// `DirEntriesList` provides a flexible way to return a list of directory entries
/// (where each entry is wrapped in a `DirEntryContainer`) from `Filesystem::readdir`.
/// It allows the entire list of containers to be borrowed, owned, or shared, providing
/// flexibility for filesystem implementations to optimize data handling.
///
/// The lifetimes are:
/// - `'dir`: Lifetime of the slice itself if `Borrowed`.
/// - `'entry`: Lifetime of borrowed `DirEntryData` within a `DirEntryContainer`.
/// - `'name`: Lifetime of borrowed names (`OsStr`) within `DirEntryData`.
#[derive(Debug)]
pub enum DirEntriesList<'dir, 'entry, 'name> {
    // 'dir is the lifetime of a borrowed list of directory entries: DirEntries.
    // 'entry is the lifetime of a borrowed entry: DirEntry.
    /// A borrowed slice of `DirEntryContainer`s. This is suitable when the list of
    /// directory entry containers is already available with a lifetime `'dir`.
    Borrowed(&'dir [DirEntryContainer<'entry, 'name>]),
    /// An owned, heap-allocated slice of `DirEntryContainer`s. This is used when
    /// the list of entries is created dynamically for the current request.
    Owned(Box<[DirEntryContainer<'entry, 'name>]>),
    /// A shared, atomically reference-counted slice of `DirEntryContainer`s. This allows
    /// the list to be shared across multiple contexts or to outlive the immediate request.
    Shared(Arc<[DirEntryContainer<'entry, 'name>]>),
}

impl<'entry, 'name> AsRef<[DirEntryContainer<'entry, 'name>]> for DirEntriesList<'_, 'entry, 'name> {
    fn as_ref(&self) -> &[DirEntryContainer<'entry, 'name>] {
        match self {
            DirEntriesList::Borrowed(s) => s,
            DirEntriesList::Owned(b) => b.as_ref(),
            DirEntriesList::Shared(a) => a.as_ref(),
        }
    }
}

impl<'entry, 'name> Deref for DirEntriesList<'_, 'entry, 'name> {
    type Target = [DirEntryContainer<'entry, 'name>];
    fn deref(&self) -> &Self::Target { self.as_ref() }
}

impl<'entry, 'name> From<Vec<DirEntryContainer<'entry, 'name>>> for DirEntriesList<'_, 'entry, 'name> {
    fn from(vec: Vec<DirEntryContainer<'entry, 'name>>) -> Self {
        DirEntriesList::Owned(vec.into_boxed_slice())
    }
}

impl<'entry, 'name> From<Box<[DirEntryContainer<'entry, 'name>]>> for DirEntriesList<'_, 'entry, 'name> {
    fn from(b: Box<[DirEntryContainer<'entry, 'name>]>) -> Self {
        DirEntriesList::Owned(b)
    }
}

impl<'entry, 'name> From<Arc<[DirEntryContainer<'entry, 'name>]>> for DirEntriesList<'_, 'entry, 'name> {
    fn from(a: Arc<[DirEntryContainer<'entry, 'name>]>) -> Self {
        DirEntriesList::Shared(a)
    }
}

impl<'entry, 'name> From<Vec<DirEntryData<'name>>> for DirEntriesList<'_, 'entry, 'name> {
    fn from(vec: Vec<DirEntryData<'name>>) -> Self {
        let boxed_slice = vec.into_iter()
            .map(DirEntryContainer::Owned)
            .collect::<Box<[DirEntryContainer<'entry, 'name>]>>();
        DirEntriesList::Owned(boxed_slice)
    }
}

impl<'entry, 'name> From<Vec<Arc<DirEntryData<'name>>>> for DirEntriesList<'_, 'entry, 'name> {
    fn from(vec: Vec<Arc<DirEntryData<'name>>>) -> Self {
        let boxed_slice = vec.into_iter()
            .map(DirEntryContainer::Shared)
            .collect::<Box<[DirEntryContainer<'entry, 'name>]>>();
        DirEntriesList::Owned(boxed_slice)
    }
}

// --- Types for readdirplus ---

/// Data for a single directory entry, including its attributes, for `readdirplus`.
/// This structure combines the basic directory entry information (`DirEntryData`)
/// with its full FUSE attributes (`fuser::Entry`).
///
/// The `'name` lifetime parameter is associated with the `name` field within
/// `entry_data` if that name is a borrowed `OsStr`.
#[derive(Debug, Clone)]
pub struct DirEntryPlusData<'name> {
    /// The core directory entry data (inode, offset, kind, name).
    /// The name within `entry_data` is an `OsBox<'name>`.
    pub entry_data: DirEntryData<'name>,
    /// The full attributes of the directory entry, as defined by `fuser::Entry`.
    /// This includes metadata like size, permissions, timestamps, etc.
    pub attr_entry: FuserEntry,
}

/// `DirEntryPlusContainer` allows a single "plus" directory entry (`DirEntryPlusData`)
/// to be returned with flexible ownership, similar to `DirEntryContainer`.
/// This is used for entries returned by `Filesystem::readdirplus`.
///
/// The lifetimes are:
/// - `'entry`: Lifetime of the `DirEntryPlusData` if `Borrowed`.
/// - `'name`: Lifetime of borrowed names (`OsStr`) within the nested `DirEntryData`.
#[derive(Debug, Clone)]
pub enum DirEntryPlusContainer<'entry, 'name> {
    /// A borrowed `DirEntryPlusData` struct. The lifetime `'entry` applies to this borrow.
    Borrowed(&'entry DirEntryPlusData<'name>),
    /// An owned `DirEntryPlusData` struct.
    Owned(DirEntryPlusData<'name>),
    /// A shared, atomically reference-counted `DirEntryPlusData` struct.
    Shared(Arc<DirEntryPlusData<'name>>),
}

impl<'name> AsRef<DirEntryPlusData<'name>> for DirEntryPlusContainer<'_, 'name> {
    fn as_ref(&self) -> &DirEntryPlusData<'name> {
        match self {
            DirEntryPlusContainer::Borrowed(data) => data,
            DirEntryPlusContainer::Owned(data) => data,
            DirEntryPlusContainer::Shared(arc_data) => arc_data.as_ref(),
        }
    }
}

impl<'name> Deref for DirEntryPlusContainer<'_, 'name> {
    type Target = DirEntryPlusData<'name>;
    fn deref(&self) -> &Self::Target { self.as_ref() }
}

impl<'entry, 'name> From<&'entry DirEntryPlusData<'name>> for DirEntryPlusContainer<'entry, 'name> {
    fn from(data_ref: &'entry DirEntryPlusData<'name>) -> Self {
        DirEntryPlusContainer::Borrowed(data_ref)
    }
}

impl<'name> From<DirEntryPlusData<'name>> for DirEntryPlusContainer<'_, 'name> {
    fn from(data: DirEntryPlusData<'name>) -> Self {
        DirEntryPlusContainer::Owned(data)
    }
}

impl<'name> From<Arc<DirEntryPlusData<'name>>> for DirEntryPlusContainer<'_, 'name> {
    fn from(arc_data: Arc<DirEntryPlusData<'name>>) -> Self {
        DirEntryPlusContainer::Shared(arc_data)
    }
}

/// `DirEntryPlusList` provides a flexible way to return a list of "plus" directory entries
/// (where each entry is a `DirEntryPlusContainer`) from `Filesystem::readdirplus`.
/// This allows the entire list of "plus" containers to be borrowed, owned, or shared.
///
/// The lifetimes are:
/// - `'dir`: Lifetime of the slice itself if `Borrowed`.
/// - `'entry`: Lifetime of borrowed `DirEntryPlusData` within a `DirEntryPlusContainer`.
/// - `'name`: Lifetime of borrowed names (`OsStr`) within the nested `DirEntryData`.
#[derive(Debug)]
pub enum DirEntryPlusList<'dir, 'entry, 'name> {
    /// A borrowed slice of `DirEntryPlusContainer`s. This is suitable when the list of
    /// "plus" directory entry containers is already available with a lifetime `'dir`.
    Borrowed(&'dir [DirEntryPlusContainer<'entry, 'name>]),
    /// An owned, heap-allocated slice of `DirEntryPlusContainer`s. This is used when
    /// the list of "plus" entries is created dynamically for the current request.
    Owned(Box<[DirEntryPlusContainer<'entry, 'name>]>),
    /// A shared, atomically reference-counted slice of `DirEntryPlusContainer`s. This allows
    /// the list to be shared across multiple contexts or to outlive the immediate request.
    Shared(Arc<[DirEntryPlusContainer<'entry, 'name>]>),
}

impl<'entry, 'name> AsRef<[DirEntryPlusContainer<'entry, 'name>]> for DirEntryPlusList<'_, 'entry, 'name> {
    fn as_ref(&self) -> &[DirEntryPlusContainer<'entry, 'name>] {
        match self {
            DirEntryPlusList::Borrowed(s) => s,
            DirEntryPlusList::Owned(b) => b.as_ref(),
            DirEntryPlusList::Shared(a) => a.as_ref(),
        }
    }
}

impl<'entry, 'name> Deref for DirEntryPlusList<'_, 'entry, 'name> {
    type Target = [DirEntryPlusContainer<'entry, 'name>];
    fn deref(&self) -> &Self::Target { self.as_ref() }
}

impl<'entry, 'name> From<Vec<DirEntryPlusContainer<'entry, 'name>>> for DirEntryPlusList<'_, 'entry, 'name> {
    fn from(vec: Vec<DirEntryPlusContainer<'entry, 'name>>) -> Self {
        DirEntryPlusList::Owned(vec.into_boxed_slice())
    }
}

impl<'entry, 'name> From<Box<[DirEntryPlusContainer<'entry, 'name>]>> for DirEntryPlusList<'_, 'entry, 'name> {
    fn from(b: Box<[DirEntryPlusContainer<'entry, 'name>]>) -> Self {
        DirEntryPlusList::Owned(b)
    }
}

impl<'entry, 'name> From<Arc<[DirEntryPlusContainer<'entry, 'name>]>> for DirEntryPlusList<'_, 'entry, 'name> {
    fn from(a: Arc<[DirEntryPlusContainer<'entry, 'name>]>) -> Self {
        DirEntryPlusList::Shared(a)
    }
}

impl<'entry, 'name> From<Vec<(DirEntryData<'name>, crate::reply::Entry)>>
    for DirEntryPlusList<'_, 'entry, 'name> {
    fn from(vec: Vec<(DirEntryData<'name>, crate::reply::Entry)>) -> Self {
        let boxed_slice = vec.into_iter()
        .map( | tuple | DirEntryPlusData{entry_data: tuple.0, attr_entry: tuple.1})
        .map(DirEntryPlusContainer::Owned)
        .collect::<Box<[DirEntryPlusContainer<'entry, 'name>]>>();
        DirEntryPlusList::Owned(boxed_slice)
    }
}

#[cfg(test)]
mod tests_byte_box { // Renamed from "tests" to be specific
    use super::*; // For ByteBox
    use std::sync::Arc; // Already here

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
        let byte_box = ByteBox::from(data_vec.clone());
        match byte_box {
            ByteBox::Owned(ref b) => assert_eq!(b.as_ref(), data_vec.as_slice()),
            _ => panic!("Expected ByteBox::Owned"),
        }
        assert_eq!(byte_box.as_ref(), data_vec.as_slice());
    }

    #[test]
    fn byte_box_owned_from_box() {
        let data_box: Box<[u8]> = vec![7, 8, 9].into_boxed_slice();
        let byte_box = ByteBox::from(data_box.clone());
        match byte_box {
            ByteBox::Owned(ref b) => assert_eq!(b.as_ref(), data_box.as_ref()),
            _ => panic!("Expected ByteBox::Owned"),
        }
        assert_eq!(byte_box.as_ref(), data_box.as_ref());
    }

    #[test]
    fn byte_box_shared_from_arc() {
        let data_arc: Arc<[u8]> = Arc::new([10, 11, 12]);
        let byte_box = ByteBox::from(data_arc.clone());
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


#[cfg(test)]
mod tests_os_box {
    use super::*; // For OsBox
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
        let os_box = OsBox::from(data_os_string.clone());
        match os_box {
            OsBox::Owned(ref b) => assert_eq!(b.as_ref(), data_os_string.as_os_str()),
            _ => panic!("Expected OsBox::Owned from OsString"),
        }
        assert_eq!(os_box.as_ref(), data_os_string.as_os_str());
    }

    #[test]
    fn os_box_owned_from_box_os_str() {
        let data_box: Box<OsStr> = OsString::from("test_owned_box").into_boxed_os_str();
        let os_box = OsBox::from(data_box.clone());
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
        if let OsBox::Owned(ref b_val) = o2 {
            assert_eq!(b_val.as_ref(), owned_os_string.as_os_str());
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

#[cfg(test)]
mod tests_dir_entry_containers {
    use super::*;
    use crate::{FileAttr, FileType, reply::DirEntryData, Entry as FuserEntry, OsBox};
    use std::ffi::{OsStr, OsString};
    use std::sync::Arc;
    use std::time::{Duration, UNIX_EPOCH};

    fn create_dir_entry_data(ino: u64, name: OsBox<'_>) -> DirEntryData<'_> {
        DirEntryData {
            ino,
            offset: ino as i64,
            kind: FileType::RegularFile,
            name,
        }
    }

    fn dummy_file_attr(ino: u64) -> FileAttr {
        FileAttr {
            ino, size: 0, blocks: 0, atime: UNIX_EPOCH, mtime: UNIX_EPOCH,
            ctime: UNIX_EPOCH, crtime: UNIX_EPOCH, kind: FileType::RegularFile,
            perm: 0, nlink: 0, uid: 0, gid: 0, rdev: 0, blksize: 0, flags: 0,
        }
    }

    #[allow(dead_code)]
    fn create_fuser_entry(attr: FileAttr) -> FuserEntry {
        FuserEntry {
            attr,
            ttl: Duration::from_secs(1),
            generation: 0,
        }
    }

    #[test]
    fn dir_entry_container_borrowed() {
        let name_os_str = OsStr::new("borrowed_entry_name");
        let entry_data = create_dir_entry_data(1, OsBox::Borrowed(name_os_str));
        let container = DirEntryContainer::Borrowed(&entry_data);
        assert_eq!(container.as_ref().ino, 1);
        assert_eq!(container.as_ref().name.as_ref(), name_os_str);
    }

    #[test]
    fn dir_entry_container_owned() {
        let owned_name = OsString::from("owned_entry_name");
        let entry_data = create_dir_entry_data(2, OsBox::Owned(owned_name.clone().into_boxed_os_str()));
        let container = DirEntryContainer::Owned(entry_data.clone());
        assert_eq!(container.as_ref().ino, 2);
        assert_eq!(container.as_ref().name.as_ref(), owned_name.as_os_str());

        let entry_data_for_from = create_dir_entry_data(2, OsBox::Owned(owned_name.into_boxed_os_str()));
        let container_from: DirEntryContainer<'static, 'static> = DirEntryContainer::from(entry_data_for_from);
        assert_eq!(container_from.as_ref().ino, 2);
    }

    #[test]
    fn dir_entry_container_shared() {
        let shared_name_os_str = OsString::from("shared_entry_name");
        let shared_name_arc: Arc<OsStr> = Arc::from(shared_name_os_str.into_boxed_os_str());
        let entry_data = create_dir_entry_data(3, OsBox::Shared(shared_name_arc.clone()));
        let arc_entry_data = Arc::new(entry_data);

        let container = DirEntryContainer::Shared(arc_entry_data.clone());
        assert_eq!(container.as_ref().ino, 3);
        assert!(Arc::ptr_eq(
            match &container.as_ref().name { OsBox::Shared(a) => a, _ => panic!()},
            &shared_name_arc
        ));

        let container_from: DirEntryContainer<'static, 'static> = DirEntryContainer::from(arc_entry_data);
        assert_eq!(container_from.as_ref().ino, 3);
    }

    #[test]
    fn dir_entries_list_borrowed() {
        let name1_static = OsStr::new("entry1_static_name");
        let entry_data1 = create_dir_entry_data(10, OsBox::Borrowed(name1_static));
        let container1 = DirEntryContainer::Borrowed(&entry_data1);

        let name2_owned = OsString::from("entry2_owned_name");
        let entry_data2 = create_dir_entry_data(11, OsBox::Owned(name2_owned.into_boxed_os_str()));
        let container2_owned = DirEntryContainer::Owned(entry_data2);

        let containers_slice: &[DirEntryContainer<'_, 'static>] = &[container1, container2_owned];
        let list = DirEntriesList::Borrowed(containers_slice);

        assert_eq!(list.len(), 2);
        assert_eq!(list[0].as_ref().ino, 10);
        assert_eq!(list[1].as_ref().ino, 11);
        assert_eq!(list[0].as_ref().name.as_ref(), name1_static);
        assert_eq!(list[1].as_ref().name.as_ref(), OsStr::new("entry2_owned_name"));
    }

    #[test]
    fn dir_entries_list_owned_from_vec() {
        let name1_static: &'static OsStr = OsStr::new("static_name_in_vec");
        let entry_data1 = create_dir_entry_data(20, OsBox::Borrowed(name1_static));
        let container1: DirEntryContainer<'static, 'static> = DirEntryContainer::Owned(entry_data1);

        let name2_owned = OsString::from("owned_name_in_vec");
        let entry_data2 = create_dir_entry_data(21, OsBox::Owned(name2_owned.into_boxed_os_str()));
        let container2: DirEntryContainer<'static, 'static> = DirEntryContainer::Owned(entry_data2);

        let containers_vec: Vec<DirEntryContainer<'static, 'static>> = vec![container1, container2];
        let list = DirEntriesList::from(containers_vec);

        match list {
            DirEntriesList::Owned(ref b_slice) => {
                assert_eq!(b_slice.len(), 2);
                assert_eq!(b_slice[0].as_ref().ino, 20);
                assert_eq!(b_slice[1].as_ref().ino, 21);
            }
            _ => panic!("Expected DirEntriesList::Owned"),
        }
    }

    #[test]
    fn dir_entries_list_shared_from_arc() {
        let name1_static: &'static OsStr = OsStr::new("shared_static_name");
        let entry_data1 = create_dir_entry_data(30, OsBox::Borrowed(name1_static));
        let container1: DirEntryContainer<'static, 'static> = DirEntryContainer::Owned(entry_data1);

        let name2_owned = OsString::from("shared_owned_name");
        let entry_data2 = create_dir_entry_data(31, OsBox::Owned(name2_owned.into_boxed_os_str()));
        let container2: DirEntryContainer<'static, 'static> = DirEntryContainer::Owned(entry_data2);

        let containers_vec: Vec<DirEntryContainer<'static, 'static>> = vec![container1, container2];
        let arc_slice: Arc<[DirEntryContainer<'static, 'static>]> = Arc::from(containers_vec);
        let list = DirEntriesList::from(arc_slice.clone());

        match list {
            DirEntriesList::Shared(ref a_slice) => {
                assert_eq!(a_slice.len(), 2);
                assert!(Arc::ptr_eq(a_slice, &arc_slice));
            }
            _ => panic!("Expected DirEntriesList::Shared"),
        }
    }

    // TODO: Add tests for DirEntryPlusData, DirEntryPlusContainer, DirEntryPlusList

    #[test]
    fn dir_entry_plus_container_owned() {
        let name_os_string = OsString::from("plus_owned_name");
        let entry_data = create_dir_entry_data(100, OsBox::Owned(name_os_string.into_boxed_os_str()));
        let attr_entry = create_fuser_entry(dummy_file_attr(100));
        let plus_data = DirEntryPlusData { entry_data, attr_entry: attr_entry.clone() };

        let container = DirEntryPlusContainer::Owned(plus_data.clone());
        assert_eq!(container.as_ref().entry_data.ino, 100);
        assert_eq!(container.as_ref().attr_entry.attr.ino, 100);
        assert_eq!(container.as_ref().entry_data.name.as_ref(), OsStr::new("plus_owned_name"));

        // Test From<DirEntryPlusData>
        let container_from: DirEntryPlusContainer<'static, 'static> = DirEntryPlusContainer::from(plus_data);
        assert_eq!(container_from.as_ref().entry_data.ino, 100);
    }

    #[test]
    fn dir_entry_plus_list_borrowed() {
        let name1_static = OsStr::new("plus_entry1_static");
        let entry_data1 = create_dir_entry_data(110, OsBox::Borrowed(name1_static));
        let attr_entry1 = create_fuser_entry(dummy_file_attr(110));
        let plus_data1 = DirEntryPlusData { entry_data: entry_data1, attr_entry: attr_entry1 };
        let container1 = DirEntryPlusContainer::Borrowed(&plus_data1);

        let name2_owned = OsString::from("plus_entry2_owned");
        let entry_data2 = create_dir_entry_data(111, OsBox::Owned(name2_owned.into_boxed_os_str()));
        let attr_entry2 = create_fuser_entry(dummy_file_attr(111));
        let plus_data2 = DirEntryPlusData { entry_data: entry_data2, attr_entry: attr_entry2 };
        let container2_owned = DirEntryPlusContainer::Owned(plus_data2);

        let containers_slice: &[DirEntryPlusContainer<'_, 'static>] = &[container1, container2_owned];
        let list = DirEntryPlusList::Borrowed(containers_slice);

        assert_eq!(list.len(), 2);
        assert_eq!(list[0].as_ref().entry_data.ino, 110);
        assert_eq!(list[1].as_ref().entry_data.ino, 111);
    }

    #[test]
    fn dir_entry_plus_list_owned_from_vec() {
        let name1_static: &'static OsStr = OsStr::new("plus_vec_static_name");
        let entry_data1 = create_dir_entry_data(120, OsBox::Borrowed(name1_static));
        let attr_entry1 = create_fuser_entry(dummy_file_attr(120));
        let plus_data1 = DirEntryPlusData { entry_data: entry_data1, attr_entry: attr_entry1 };
        let container1: DirEntryPlusContainer<'static, 'static> = DirEntryPlusContainer::Owned(plus_data1);

        let name2_owned = OsString::from("plus_vec_owned_name");
        let entry_data2 = create_dir_entry_data(121, OsBox::Owned(name2_owned.into_boxed_os_str()));
        let attr_entry2 = create_fuser_entry(dummy_file_attr(121));
        let plus_data2 = DirEntryPlusData { entry_data: entry_data2, attr_entry: attr_entry2 };
        let container2: DirEntryPlusContainer<'static, 'static> = DirEntryPlusContainer::Owned(plus_data2);

        let containers_vec: Vec<DirEntryPlusContainer<'static, 'static>> = vec![container1, container2];
        let list = DirEntryPlusList::from(containers_vec);

        match list {
            DirEntryPlusList::Owned(ref b_slice) => {
                assert_eq!(b_slice.len(), 2);
                assert_eq!(b_slice[0].as_ref().entry_data.ino, 120);
                assert_eq!(b_slice[1].as_ref().entry_data.ino, 121);
            }
            _ => panic!("Expected DirEntryPlusList::Owned"),
        }
    }
}
