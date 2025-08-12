// This example requires fuse 7.11 or later. Run with:
//
//   cargo run --example ioctl --features abi-7-11 /tmp/foobar

#![allow(clippy::cast_possible_truncation)] // many conversions with unhandled errors

use async_trait::async_trait;
use bytes::Bytes;
use clap::{Arg, ArgAction, Command, crate_version};
use fuser::{
    Dirent, DirentList, Entry, Errno, FileAttr, FileType, Ioctl, MountOption, RequestMeta,
    trait_async::Filesystem,
};
use log::{debug, info};
use std::ffi::OsStr;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, UNIX_EPOCH};

const TTL: Duration = Duration::from_secs(1); // 1 second

const FIOC_GET_SIZE: u64 = nix::request_code_read!('E', 0, std::mem::size_of::<usize>());
const FIOC_SET_SIZE: u64 = nix::request_code_write!('E', 1, std::mem::size_of::<usize>());

struct FiocFS {
    content: Mutex<Vec<u8>>,
    root_attr: FileAttr,
    fioc_file_attr: FileAttr,
}

static DIRENTS: [Dirent; 3] = [
    Dirent {
        ino: 1,
        offset: 1,
        kind: FileType::Directory,
        name: Bytes::from_static(b"."),
    },
    Dirent {
        ino: 1,
        offset: 2,
        kind: FileType::Directory,
        name: Bytes::from_static(b".."),
    },
    Dirent {
        ino: 2,
        offset: 3,
        kind: FileType::RegularFile,
        name: Bytes::from_static(b"fioc"),
    },
];

impl FiocFS {
    fn new() -> Self {
        let uid = unsafe { libc::getuid() };
        let gid = unsafe { libc::getgid() };

        let root_attr = FileAttr {
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
            uid,
            gid,
            rdev: 0,
            flags: 0,
            blksize: 512,
        };

        let fioc_file_attr = FileAttr {
            ino: 2,
            size: 0,
            blocks: 1,
            atime: UNIX_EPOCH, // 1970-01-01 00:00:00
            mtime: UNIX_EPOCH,
            ctime: UNIX_EPOCH,
            crtime: UNIX_EPOCH,
            kind: FileType::RegularFile,
            perm: 0o644,
            nlink: 1,
            uid,
            gid,
            rdev: 0,
            flags: 0,
            blksize: 512,
        };

        Self {
            content: Mutex::new(vec![]),
            root_attr,
            fioc_file_attr,
        }
    }
}

#[async_trait]
impl Filesystem for FiocFS {
    async fn lookup(&self, _req: RequestMeta, parent: u64, name: &Path) -> Result<Entry, Errno> {
        if parent == 1 && name == OsStr::new("fioc") {
            Ok(Entry {
                ino: self.fioc_file_attr.ino,
                generation: None,
                attr: self.fioc_file_attr,
                attr_ttl: TTL,
                file_ttl: TTL,
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
            1 => Ok((self.root_attr, TTL)),
            2 => Ok((self.fioc_file_attr, TTL)),
            _ => Err(Errno::ENOENT),
        }
    }

    #[allow(clippy::cast_sign_loss)]
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
            let offset = offset as usize;
            if offset >= self.content.lock().unwrap().len() {
                // No need to allocate anything: just use `Bytes::new()`, which does not allocate.
                Ok(Bytes::new())
            } else {
                // Using `From` trait to construct onwed Bytes
                Ok(self.content.lock().unwrap()[offset..].to_vec().into())
            }
        } else {
            Err(Errno::ENOENT)
        }
    }

    #[allow(clippy::cast_possible_truncation)]
    async fn readdir(
        &self,
        _req: RequestMeta,
        ino: u64,
        _fh: u64,
        // Fuser library will ensure that offset and max_bytes are respected.
        _offset: i64,
        _max_bytes: u32,
    ) -> Result<DirentList, Errno> {
        if ino != 1 {
            return Err(Errno::ENOENT);
        }
        // Static data can be borrowed to prevent unnecessary copying
        // In this example, return a borrowed reference to the (static) list of dir entries.
        Ok(DirentList::Static(&DIRENTS))
    }

    #[cfg(feature = "abi-7-11")]
    async fn ioctl(
        &self,
        _req: RequestMeta,
        ino: u64,
        _fh: u64,
        _flags: u32,
        cmd: u32,
        in_data: &[u8],
        _out_size: u32,
    ) -> Result<Ioctl, Errno> {
        if ino != 2 {
            return Err(Errno::EINVAL);
        }

        match cmd.into() {
            FIOC_GET_SIZE => {
                let size_bytes = self.content.lock().unwrap().len().to_ne_bytes();
                Ok(Ioctl {
                    result: 0,
                    data: Bytes::from(size_bytes.to_vec()),
                })
            }
            FIOC_SET_SIZE => {
                let new_size = usize::from_ne_bytes(in_data.try_into().unwrap());
                *self.content.lock().unwrap() = vec![0_u8; new_size];
                Ok(Ioctl {
                    result: 0,
                    data: Bytes::new(),
                })
            }
            _ => {
                debug!("unknown ioctl: {cmd}");
                Err(Errno::EINVAL)
            }
        }
    }
}

fn main() {
    let matches = Command::new("hello")
        .version(crate_version!())
        .author("Colin Marc")
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
    let mut options = vec![MountOption::FSName("fioc".to_string())];
    if matches.get_flag("auto_unmount") {
        options.push(MountOption::AutoUnmount);
    }
    if matches.get_flag("allow-root") {
        options.push(MountOption::AllowRoot);
    }

    let fs = FiocFS::new();
    // fuser::mount2(fs, mountpoint, &options).unwrap();
    let se = fuser::Session::new_mounted(fs.into(), mountpoint, &options)
        .expect("Failed to create Session");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    match rt.block_on(se.run_async()) {
        Ok(_se) => info!("Session ended safely"),
        Err(e) => info!("Session ended with error {e:?}"),
    }
}
