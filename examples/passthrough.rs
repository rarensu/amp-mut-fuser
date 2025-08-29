// This example requires fuse 7.40 or later.
//
// To run this example, do the following:
//
//     sudo RUST_LOG=info ./target/debug/examples/passthrough /tmp/mnt &
//     sudo cat /tmp/mnt/passthrough
//     sudo pkill passthrough
//     sudo umount /tmp/mnt

use bytes::Bytes;
use clap::{Arg, ArgAction, Command, crate_version};
use fuser::{
    BackingId, BackingHandler, Dirent, DirentList, Entry, Errno, FileAttr, FileType,
    FsStatus, KernelConfig, MountOption, Open, RequestMeta, consts, trait_async::Filesystem,
};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::File;
use std::path::Path;
use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const TTL: Duration = Duration::from_secs(1); // 1 second
const BACKING_TIMEOUT: Duration = Duration::from_secs(2); // 2 seconds

#[derive(Debug)]
struct PassthroughFs {
    root_attr: FileAttr,
    passthrough_file_attr: FileAttr,
    backing_cache: Mutex<HashMap<u64, (BackingId, SystemTime)>>,
    next_fh: AtomicU64,
    backing_handler: BackingHandler,
}

static ROOT_DIRENTS: [Dirent; 3] = [
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
        name: Bytes::from_static(b"passthrough"),
    },
];

impl PassthroughFs {
    fn new(backing_handler: BackingHandler) -> Self {
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

        let passthrough_file_attr = FileAttr {
            ino: 2,
            size: 123456,
            blocks: 1,
            atime: UNIX_EPOCH, // 1970-01-01 00:00:00
            mtime: UNIX_EPOCH,
            ctime: UNIX_EPOCH,
            crtime: UNIX_EPOCH,
            kind: FileType::RegularFile,
            perm: 0o644,
            nlink: 1,
            uid: 333,
            gid: 333,
            rdev: 0,
            flags: 0,
            blksize: 512,
        };

        Self {
            root_attr,
            passthrough_file_attr,
            backing_cache: Mutex::new(HashMap::new()),
            next_fh: AtomicU64::new(0),
            backing_handler,
        }
    }

    fn next_fh(&self) -> u64 {
        self.next_fh.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn get_backing_id(&self, ino: u64) -> Option<u32> {
        self.update_backing_status(ino, true);
        if let Some((ready_backing_id, _)) =
            self.backing_cache.lock().unwrap().get(&ino)
        {
            Some(ready_backing_id.id)
        } else {
            None
        }
    }


    // Get the backing status for a given inode, after all available updates are applied.
    // This will advance the status as appropriate and remove it from the cache if it is stale.
    // The returning an immutable reference to the updated status (or None)
    fn update_backing_status(&self, ino: u64, extend: bool) {
        let mut cache = self.backing_cache.lock().unwrap();
        let mut remove = false;
        if let Some((backing_id, creation_time)) = cache.get_mut(&ino) {
            let now = SystemTime::now();
            if extend {
                *creation_time = now;
                log::info!("Backing Id {} Timestamp Renewed", backing_id.id);
            } else if now.duration_since(creation_time.clone()).unwrap() > BACKING_TIMEOUT {
                log::info!("Backing Id {} Timed Out", backing_id.id);
                remove = true;
            }
        }
        if remove {
            cache.remove(&ino);
        }
    }
}

use async_trait::async_trait;

#[async_trait]
impl Filesystem for PassthroughFs {
    async fn init(&self, _req: RequestMeta, config: KernelConfig) -> Result<KernelConfig, Errno> {
        let mut config = config;
        config.add_capabilities(consts::FUSE_PASSTHROUGH).expect(
            "FUSE Kernel did not advertise support for passthrough (required for this example).",
        );
        config.set_max_stack_depth(2).unwrap();
        Ok(config)
    }

    async fn lookup(&self, _req: RequestMeta, parent: u64, name: &OsStr) -> Result<Entry, Errno> {
        log::info!("lookup(name={name:?})");
        if parent == 1 && name.to_str() == Some("passthrough") {
            Ok(Entry {
                ino: self.passthrough_file_attr.ino,
                generation: None,
                file_ttl: TTL,
                attr: self.passthrough_file_attr,
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
            1 => Ok((self.root_attr, TTL)),
            2 => Ok((self.passthrough_file_attr, TTL)),
            _ => Err(Errno::ENOENT),
        }
    }

    async fn open(&self, _req: RequestMeta, ino: u64, _flags: i32) -> Result<Open, Errno> {
        if ino != 2 {
            return Err(Errno::ENOENT);
        }
        // Check if a backing id is ready for this file
        let backing_id_option = if let Some(id) = self.get_backing_id(ino){
            Some(id)
        } else {
            // Try to obtain a backing id
            let file = File::open("/etc/profile")?;
            let backing_id = self.backing_handler.open_backing(file)?;
            let id = backing_id.id;
            let now = SystemTime::now();
            self.backing_cache.lock().unwrap().insert(ino, (backing_id, now));
            Some(id)
        };
        let fh = self.next_fh();
        // TODO: track file handles
        log::info!("open: fh {fh}, backing_id_option {backing_id_option:?}");
        Ok(Open {
            fh,
            flags: consts::FOPEN_PASSTHROUGH,
            backing_id: backing_id_option,
        })
    }

    // The heartbeat function is called periodically by the FUSE session.
    // We use it to ensure that the cache entries have accurate timestamps.
    async fn heartbeat(&self) -> FsStatus {
        let keys: Vec<u64> = {
            let cache = self.backing_cache.lock().unwrap();
            cache.keys().copied().collect()
        };
        for ino in keys {
            self.update_backing_status(ino, false);
        }
        FsStatus::Ready
    }

    // This deliberately unimplemented read() function proves that the example demonstrates passthrough.
    // If a user is able to read the file, it could only have been via the kernel.
    async fn read(
        &self,
        _req: RequestMeta,
        _ino: u64,
        _fh: u64,
        _offset: i64,
        _size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
    ) -> Result<Bytes, Errno> {
        unimplemented!();
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
        // In this example, return up to three entries depending on the offset.
        if (0..=2).contains(&offset) {
            // Case: offset in range:
            // Return a borrowed ('static) slice of entries.
            Ok(DirentList::Static(&ROOT_DIRENTS[offset as usize..]))
        } else {
            // Case: offset out of range:
            // No need to allocate anything; just use the Empty enum case.
            Ok(DirentList::Empty)
        }
    }

    async fn release(
        &self,
        _req: RequestMeta,
        _ino: u64,
        _fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
    ) -> Result<(), Errno> {
        // TODO: mark fh as unused
        Ok(())
    }
}

fn main() {
    let matches = Command::new("hello")
        .version(crate_version!())
        .author("Allison Karlitskaya")
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
    let mut options = vec![MountOption::FSName("passthrough".to_string())];
    if matches.get_flag("auto_unmount") {
        options.push(MountOption::AutoUnmount);
    }
    if matches.get_flag("allow-root") {
        options.push(MountOption::AllowRoot);
    }

    let mut sessionbuilder = fuser::SessionBuilder::new();
    sessionbuilder.mount_path(Path::new(mountpoint), &options)
            .expect("Failed to mount FUSE session.");
    sessionbuilder.set_heartbeat_interval(Duration::new(1, 0));
    let backing_handler = sessionbuilder.get_backing_handler();
    let fs = PassthroughFs::new(backing_handler);
    sessionbuilder.set_filesystem(fs.into());
    let session = sessionbuilder.build();

    // Drive the async session loop with a Tokio runtime, matching ioctl.rs.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    match rt.block_on(session.run_async()) {
        Ok(()) => log::info!("Session ended safely"),
        Err(e) => log::info!("Session ended with error: {e:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn dummy_meta() -> RequestMeta {
        RequestMeta {
            unique: 0,
            uid: 1000,
            gid: 1000,
            pid: 2000,
        }
    }

    #[test]
    fn test_something() {
    }
}
