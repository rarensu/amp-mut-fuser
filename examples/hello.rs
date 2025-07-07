use clap::{crate_version, Arg, ArgAction, Command};
use fuser::{
    Attr, ByteBox, DirEntriesList, DirEntryContainer, DirEntryData, Entry, Errno, FileAttr,
    Filesystem, FileType, MountOption, OsBox, RequestMeta,
};
use std::ffi::{OsStr, OsString};
use std::time::{Duration, UNIX_EPOCH};

const TTL: Duration = Duration::from_secs(1); // 1 second

const HELLO_DIR_ATTR: FileAttr = FileAttr {
    ino: 1,
    size: 0,
    blocks: 0,
    atime: UNIX_EPOCH, // 1970-01-01 00:00:00
    mtime: UNIX_EPOCH,
    ctime: UNIX_EPOCH,
    crtime: UNIX_EPOCH,
    kind: FileType::Directory,
    perm: 0o755,
    nlink: 2,
    uid: 501,
    gid: 20,
    rdev: 0,
    flags: 0,
    blksize: 512,
};

const HELLO_TXT_CONTENT: &str = "Hello World!\n";

const HELLO_TXT_ATTR: FileAttr = FileAttr {
    ino: 2,
    size: 13,
    blocks: 1,
    atime: UNIX_EPOCH, // 1970-01-01 00:00:00
    mtime: UNIX_EPOCH,
    ctime: UNIX_EPOCH,
    crtime: UNIX_EPOCH,
    kind: FileType::RegularFile,
    perm: 0o644,
    nlink: 1,
    uid: 501,
    gid: 20,
    rdev: 0,
    flags: 0,
    blksize: 512,
};

struct HelloFS;

impl Filesystem for HelloFS {
    fn lookup(&mut self, _req: RequestMeta, parent: u64, name: OsString) -> Result<Entry, Errno> {
        if parent == 1 && name == OsStr::new("hello.txt") {
            Ok(Entry{attr: HELLO_TXT_ATTR, ttl: TTL, generation: 0})
        } else {
            Err(Errno::ENOENT)
        }
    }

    fn getattr(
        &mut self,
        _req: RequestMeta,
        ino: u64,
        _fh: Option<u64>,
    ) -> Result<Attr, Errno> {
        match ino {
            1 => Ok(Attr{attr: HELLO_DIR_ATTR, ttl: TTL,}),
            2 => Ok(Attr{attr: HELLO_TXT_ATTR, ttl: TTL,}),
            _ => Err(Errno::ENOENT),
        }
    }

    fn read<'a>(
        &mut self,
        _req: RequestMeta,
        ino: u64,
        _fh: u64,
        offset: i64,
        _size: u32,
        _flags: i32,
        _lock: Option<u64>,
    ) -> Result<ByteBox<'a>, Errno> {
        if ino == 2 {
            // HELLO_TXT_CONTENT is &'static str, so its bytes are &'static [u8]
            // We can borrow this directly.
            let bytes = HELLO_TXT_CONTENT.as_bytes();
            let slice_len = bytes.len();
            let offset = offset as usize;
            if offset >= slice_len {
                Ok(ByteBox::Borrowed(&[]))
            } else {
                Ok(ByteBox::Borrowed(&bytes[offset..]))
            }
        } else {
            Err(Errno::ENOENT)
        } // <<< Added missing brace here
    }

    fn readdir<'list_lt, 'entry_lt, 'name_lt>(
        &mut self,
        _req: RequestMeta,
        ino: u64,
        _fh: u64,
        offset: i64,
        _max_bytes: u32,
    ) -> Result<DirEntriesList<'list_lt, 'entry_lt, 'name_lt>, Errno> {
        if ino != 1 {
            return Err(Errno::ENOENT);
        }

        let mut entries: Vec<DirEntryContainer<'static, 'static>> = Vec::new();

        // Entry 1: "."
        if offset < 1 {
            entries.push(DirEntryContainer::Owned(DirEntryData {
                ino: 1,
                offset: 1, // This entry's cookie
                kind: FileType::Directory,
                name: OsBox::Borrowed(OsStr::new(".")),
            }));
        }

        // Entry 2: ".."
        if offset < 2 {
            entries.push(DirEntryContainer::Owned(DirEntryData {
                ino: 1, // Parent of root is root itself for simplicity here
                offset: 2, // This entry's cookie
                kind: FileType::Directory,
                name: OsBox::Borrowed(OsStr::new("..")),
            }));
        }

        // Entry 3: "hello.txt"
        if offset < 3 {
            entries.push(DirEntryContainer::Owned(DirEntryData {
                ino: 2,
                offset: 3, // This entry's cookie
                kind: FileType::RegularFile,
                name: OsBox::Owned(OsString::from("hello.txt").into_boxed_os_str()),
            }));
        }

        // Slice the collected entries based on the offset.
        // The FUSE convention is that offset is the cookie of the *previous* entry,
        // or 0 for the first call. So, if offset is k, we start sending from the (k)th entry (0-indexed).
        // The `offset` field in `DirEntryData` should be the cookie that `readdir` would return
        // for *this* entry, so that if `readdir` is called again with that cookie, it knows where to resume.
        // A common way is to use 1-based indexing for these cookies.

        // Let's refine the logic slightly to be more standard with offset handling.
        // The provided offset is the point *from which* to start reading.
        // If offset = 0, send all. If offset = 1, send from ".." onwards. If offset = 2, send from "hello.txt" onwards.
        // The `offset` field in `DirEntryData` should be the cookie for *that specific entry*.
        // It's often the inode number or a sequence number.

        let all_possible_entries = [
            (DirEntryData { ino: 1, offset: 1, kind: FileType::Directory, name: OsBox::Borrowed(OsStr::new(".")) }),
            (DirEntryData { ino: 1, offset: 2, kind: FileType::Directory, name: OsBox::Borrowed(OsStr::new("..")) }),
            (DirEntryData { ino: 2, offset: 3, kind: FileType::RegularFile, name: OsBox::Owned(OsString::from("hello.txt").into_boxed_os_str()) }),
        ];

        let mut result_containers: Vec<DirEntryContainer<'static, 'static>> = Vec::new();
        for entry_data in all_possible_entries.iter().skip(offset as usize) {
            // We are creating new DirEntryContainer::Owned for simplicity,
            // as the `all_possible_entries` array is local to this function call.
            // If these were truly static `DirEntryData` instances, we could use `Borrowed`.
            result_containers.push(DirEntryContainer::Owned(entry_data.clone()));
        }

        Ok(DirEntriesList::from(result_containers))
    }
}

fn main() {
    let matches = Command::new("hello")
        .version(crate_version!())
        .author("Christopher Berner")
        .arg(
            Arg::new("MOUNT_POINT")
                .required(true)
                .index(1)
                .help("Act as a client, and mount FUSE at given path"),
        )
        .arg(
            Arg::new("auto_unmount")
                .long("auto_unmount")
                .action(ArgAction::SetTrue)
                .help("Automatically unmount on process exit"),
        )
        .arg(
            Arg::new("allow-root")
                .long("allow-root")
                .action(ArgAction::SetTrue)
                .help("Allow root user to access filesystem"),
        )
        .get_matches();
    env_logger::init();
    let mountpoint = matches.get_one::<String>("MOUNT_POINT").unwrap();
    let mut options = vec![MountOption::RO, MountOption::FSName("hello".to_string())];
    if matches.get_flag("auto_unmount") {
        options.push(MountOption::AutoUnmount);
    }
    if matches.get_flag("allow-root") {
        options.push(MountOption::AllowRoot);
    }
    fuser::mount2(HelloFS, mountpoint, &options).unwrap();
}

#[cfg(test)]
mod test {
    use fuser::{Filesystem, RequestMeta, Errno, FileType};
    use std::ffi::{OsString, OsStr}; // Added OsStr

    fn dummy_meta() -> RequestMeta {
        RequestMeta { unique: 0, uid: 1000, gid: 1000, pid: 2000 }
    }

    #[test]
    fn test_lookup_hello_txt() {
        let mut hellofs = super::HelloFS {};
        let req = dummy_meta();
        let result = hellofs.lookup(req, 1, OsString::from("hello.txt"));
        assert!(result.is_ok(), "Lookup for hello.txt should succeed");
        if let Ok(entry) = result {
            assert_eq!(entry.attr.ino, 2, "Lookup should return inode 2 for hello.txt");
            assert_eq!(entry.attr.kind, FileType::RegularFile, "hello.txt should be a regular file");
            assert_eq!(entry.attr.perm, 0o644, "hello.txt should have permissions 0o644");
        }
    }

    #[test]
    fn test_read_hello_txt() {
        let mut hellofs = super::HelloFS {};
        let req = dummy_meta();
        let result = hellofs.read(req, 2, 0, 0, 13, 0, None);
        assert!(result.is_ok(), "Read for hello.txt should succeed");
        if let Ok(content) = result {
            assert_eq!(String::from_utf8_lossy(&content), "Hello World!\n", "Content of hello.txt should be 'Hello World!\\n'");
        }
    }

    #[test]
    fn test_readdir_root() {
        let mut hellofs = super::HelloFS {};
        let req = dummy_meta();
        let result = hellofs.readdir(req, 1, 0, 0, 4096);
        assert!(result.is_ok(), "Readdir on root should succeed");
        if let Ok(entries_list) = result {
            let entries_slice = entries_list.as_ref();
            assert_eq!(entries_slice.len(), 3, "Root directory should contain exactly 3 entries");

            // Check entry 0: "."
            let entry0_data = entries_slice[0].as_ref();
            assert_eq!(entry0_data.name.as_ref(), OsStr::new("."), "First entry should be '.'");
            assert_eq!(entry0_data.ino, 1, "Inode for '.' should be 1");
            assert_eq!(entry0_data.offset, 1, "Offset for '.' should be 1");
            assert_eq!(entry0_data.kind, FileType::Directory, "'.' should be a directory");

            // Check entry 1: ".."
            let entry1_data = entries_slice[1].as_ref();
            assert_eq!(entry1_data.name.as_ref(), OsStr::new(".."), "Second entry should be '..'");
            assert_eq!(entry1_data.ino, 1, "Inode for '..' should be 1");
            assert_eq!(entry1_data.offset, 2, "Offset for '..' should be 2");
            assert_eq!(entry1_data.kind, FileType::Directory, "'..' should be a directory");

            // Check entry 2: "hello.txt"
            let entry2_data = entries_slice[2].as_ref();
            assert_eq!(entry2_data.name.as_ref(), OsStr::new("hello.txt"), "Third entry should be 'hello.txt'");
            assert_eq!(entry2_data.ino, 2, "Inode for 'hello.txt' should be 2");
            assert_eq!(entry2_data.offset, 3, "Offset for 'hello.txt' should be 3");
            assert_eq!(entry2_data.kind, FileType::RegularFile, "'hello.txt' should be a regular file");
        }
    }

    #[test]
    fn test_create_fails_readonly() {
        let mut hellofs = super::HelloFS {};
        let req = dummy_meta();
        let result = hellofs.create(req, 1, OsString::from("newfile.txt"), 0o644, 0, 0);
        assert!(result.is_err(), "Create should fail for read-only filesystem");
        if let Err(e) = result {
            assert_eq!(e, Errno::ENOSYS, "Create should return ENOSYS for unsupported operation");
        }
    }
}
