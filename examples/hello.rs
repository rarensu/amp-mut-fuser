use clap::{crate_version, Arg, ArgAction, Command};
use fuser::{
    FileAttr, Dirent, DirentList, Entry, Errno,
    trait_async::Filesystem, FileType, MountOption, RequestMeta,
};
use bytes::Bytes;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use async_trait::async_trait;

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

// An example of reusable Borrowed data.
// This entry derives its lifetime from string literal, 
// which is 'static.
const DOT_DIRENT: Dirent = Dirent {
    ino: 1,
    offset: 1,
    kind: FileType::Directory,
    name: Bytes::from_static(b"."),
};

/// Example Filesystem data
struct HelloFS {
    hello_filename: Bytes,
}

impl HelloFS {

    fn new() -> Self {
        HelloFS{
            // An example of reusable Shared data.
            // The filename for Dirent #3 is allocated here once.
            // It is persistent until replaced.
            hello_filename: Bytes::from_owner(Vec::from(b"hello.txt")),
        }
    }
}

#[async_trait]
impl Filesystem for HelloFS {

    async fn lookup(&self, _req: RequestMeta, parent: u64, name: &Path) -> Result<Entry, Errno> {
        if parent == 1 && name.as_os_str().as_encoded_bytes() == self.hello_filename {
            Ok(Entry{
                ino: 2,
                generation: None,
                file_ttl: TTL,
                attr: HELLO_TXT_ATTR, 
                attr_ttl: TTL, 
            })
        } else {
            Err(Errno::ENOENT)
        }
    }

    async fn getattr(
        &self,
        _req: RequestMeta,
        ino: u64,
        _fh: Option<u64>,
    ) -> Result<(FileAttr, Duration), Errno> {
        match ino {
            1 => Ok((HELLO_DIR_ATTR, TTL)),
            2 => Ok((HELLO_TXT_ATTR, TTL)),
            _ => Err(Errno::ENOENT),
        }
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    async fn read(
        &self,
        _req: RequestMeta,
        ino: u64,
        _fh: u64,
        offset: i64,
        _size: u32,
        _flags: i32,
        _lock: Option<u64>,
    ) -> Result<Bytes, Errno> {
        if ino == 2 {
            // HELLO_TXT_CONTENT is &'static str, so its bytes are &'static [u8]
            let bytes = HELLO_TXT_CONTENT.as_bytes();
            let slice_len = bytes.len();
            let offset = offset as usize;
            if offset >= slice_len {
                Ok(Bytes::from_static(&[]))
            } else {
                // Returning as Borrowed to avoid a copy.
                Ok(Bytes::from_static(&bytes[offset..]))
            }
        } else {
            Err(Errno::ENOENT)
        }
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    async fn readdir(
        &self,
        _req: RequestMeta,
        ino: u64,
        _fh: u64,
        offset: i64,
        _max_bytes: u32,
    ) -> Result<DirentList, Errno> {
        if ino != 1 {
            return Err(Errno::ENOENT);
        }

        // This example builds the list of 3 dirents from scratch 
        // on each call to readdir().
        let mut entries= Vec::new();

        // Entry 1: example of borrowed data.
        // - name: "."
        // - Dirent is constructed in the global scope. 
        // - lifetime of the `name` field is 'static by definition.
        // - the vector will hold a reference to the `name` field.
        entries.push(DOT_DIRENT);

        // Entry 2: example of single-use Owned data.
        // - name: ".."
        // - dirent is constructed during each call to readdir(). 
        let dotdot_dirent = Dirent {
                ino: 1, // Parent of root is itself for simplicity. 
                        // Note: this can cause some weird behavior for an observer.
                offset: 2,
                kind: FileType::Directory,
                // ownership of this new byte vector is moved into the new Dirent
                name: Bytes::from_owner(Vec::from(".."))
            };
        // Ownership of the entry is passed along
        entries.push(dotdot_dirent);

        // Entry 3: an example of shared data.
        // - hello_filename is owned by HelloFS
        // - A clone of a Bytes is a reference counted pointer, not a deep copy.
        // - Therefore, the Dirent and HelloFS share ownership.
        let hello_dirent = Dirent {
                ino: 2,
                offset: 3,
                kind: FileType::RegularFile,
                // a copy of the smart pointer is moved into the Dirent
                name: self.hello_filename.clone()
            };
        entries.push(hello_dirent);

        // Slice the collected dirents based on the requested offset.
        let entries: Vec<Dirent> = entries.into_iter().skip(offset as usize).collect();
        // ( Only references and smart pointers are being reorganized at this time;
        // the underlying filename data should stay where it is.)
        
        // A DirentList itself may also be returned as borrowed, owned, or shared.
        // In this example, it is owned (Vec).
        // From<...> and Into<...> methods can be used to help construct the return type.  
        Ok(entries.into())
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
    let hellofs = HelloFS::new();
    fuser::mount2(hellofs.into(), mountpoint, &options).unwrap();
}

#[cfg(test)]
mod test {
    use fuser::{Filesystem, RequestMeta, Errno, FileType};
    use std::ffi::OsStr;
    use std::path::PathBuf;
    use std::os::unix::ffi::OsStrExt;

    fn dummy_meta() -> RequestMeta {
        RequestMeta { unique: 0, uid: 1000, gid: 1000, pid: 2000 }
    }

    #[test]
    fn test_lookup_hello_txt() {
        let hellofs = super::HelloFS::new();
        let req = dummy_meta();
        let result = futures::executor::block_on(
            hellofs.lookup(req, 1, &PathBuf::from("hello.txt"))
        );
        assert!(result.is_ok(), "Lookup for hello.txt should succeed");
        if let Ok(entry) = result {
            assert_eq!(entry.attr.ino, 2, "Lookup should return inode 2 for hello.txt");
            assert_eq!(entry.attr.kind, FileType::RegularFile, "hello.txt should be a regular file");
            assert_eq!(entry.attr.perm, 0o644, "hello.txt should have permissions 0o644");
        }
    }

    #[test]
    fn test_read_hello_txt() {
        let hellofs = super::HelloFS::new();
        let req = dummy_meta();
        let result = futures::executor::block_on(
            hellofs.read(req, 2, 0, 0, 13, 0, None)
        );
        assert!(result.is_ok(), "Read for hello.txt should succeed");
        if let Ok(content) = result {
            assert_eq!(String::from_utf8_lossy(content.as_ref()), "Hello World!\n", "Content of hello.txt should be 'Hello World!\\n'");
        }
    }

    #[test]
    fn test_readdir_root() {
        let hellofs = super::HelloFS::new();
        let req = dummy_meta();
        let result = futures::executor::block_on(
            hellofs.readdir(req, 1, 0, 0, 4096)
        );
        assert!(result.is_ok(), "Readdir on root should succeed");
        if let Ok(entries_list) = result {
            // using unlock().unwrap() in case locking variants are enabled in the current build.
            // otherwise, one could simply use as_ref() or &
            let entries_slice = entries_list.unlock().unwrap();
            assert_eq!(entries_slice.len(), 3, "Root directory should contain exactly 3 entries");

            // Check entry 0: "."
            let entry0_data = &entries_slice[0];
            assert_eq!(entry0_data.name.as_ref(), OsStr::new(".").as_bytes(), "First entry should be '.'");
            assert_eq!(entry0_data.ino, 1, "Inode for '.' should be 1");
            assert_eq!(entry0_data.offset, 1, "Offset for '.' should be 1");
            assert_eq!(entry0_data.kind, FileType::Directory, "'.' should be a directory");

            // Check entry 1: ".."
            let entry1_data = &entries_slice[1];
            assert_eq!(entry1_data.name.as_ref(), OsStr::new("..").as_bytes(), "Second entry should be '..'");
            assert_eq!(entry1_data.ino, 1, "Inode for '..' should be 1");
            assert_eq!(entry1_data.offset, 2, "Offset for '..' should be 2");
            assert_eq!(entry1_data.kind, FileType::Directory, "'..' should be a directory");

            // Check entry 2: "hello.txt"
            let entry2_data = &entries_slice[2];
            assert_eq!(entry2_data.name.as_ref(), OsStr::new("hello.txt").as_bytes(), "Third entry should be 'hello.txt'");
            assert_eq!(entry2_data.ino, 2, "Inode for 'hello.txt' should be 2");
            assert_eq!(entry2_data.offset, 3, "Offset for 'hello.txt' should be 3");
            assert_eq!(entry2_data.kind, FileType::RegularFile, "'hello.txt' should be a regular file");
        }
    }

    #[test]
    fn test_create_fails_readonly() {
        let hellofs = super::HelloFS::new();
        let req = dummy_meta();
        let result = futures::executor::block_on(
            hellofs.create(req, 1, &PathBuf::from("newfile.txt"), 0o644, 0, 0)
        );
        assert!(result.is_err(), "Create should fail for read-only filesystem");
        if let Err(e) = result {
            assert_eq!(e, Errno::ENOSYS, "Create should return ENOSYS for unsupported operation");
        }
    }
}
