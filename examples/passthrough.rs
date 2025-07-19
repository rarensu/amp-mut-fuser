// This example requires fuse 7.40 or later. Run with:
//
//   cargo run --example passthrough --features abi-7-40 /tmp/foobar

use clap::{crate_version, Arg, ArgAction, Command};
use fuser::{
    consts, Attr, DirEntry, Entry, Errno, FileAttr, FileType, Filesystem, KernelConfig,
    MountOption, Open, RequestMeta, Notification,

};
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::File;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const TTL: Duration = Duration::from_secs(1); // 1 second

// ----- BackingID -----

use std::io;

#[derive(Debug)]
struct PendingBackingId {
    #[allow(dead_code)]
    reply: crossbeam_channel::Receiver<io::Result<u32>>,
    #[allow(dead_code)]
    _file: Arc<File>,
}

#[derive(Debug)]
struct ReadyBackingId {
    notifier: crossbeam_channel::Sender<Notification>,
    backing_id: u32,
    timestamp: SystemTime,
    reply_sender: Option<crossbeam_channel::Sender<io::Result<u32>>>,
}

impl Drop for ReadyBackingId {
    fn drop(&mut self) {
        let notification = Notification::CloseBacking((self.backing_id, self.reply_sender.take()));
        let _ = self.notifier.send(notification);
    }
}

#[derive(Debug)]
struct DroppedBackingId {
    reply: crossbeam_channel::Receiver<io::Result<u32>>,
}

#[derive(Debug)]
enum BackingStatus {
    Pending(PendingBackingId),
    Ready(ReadyBackingId),
    Dropped(DroppedBackingId),
}

/// A cache for `BackingId` objects.
///
/// This cache is designed to avoid creating more than one `BackingId` per file at a time. It
/// uses a weak "by inode" hash table to map inode numbers to `BackingId`s. If a `BackingId`
/// already exists for an inode, it is reused. Otherwise, a new one is created.
///
/// The cache also maintains a strong reference to the `BackingId` for each open file handle.
/// This ensures that the `BackingId` is not dropped while it is still in use. The strong
/// reference is dropped when the file is released.
#[derive(Debug, Default)]
struct BackingCache {
    by_inode: HashMap<u64, BackingStatus>,
}

use crossbeam_channel::Sender;

#[derive(Debug)]
struct PassthroughFs {
    root_attr: FileAttr,
    passthrough_file_attr: FileAttr,
    backing_cache: BackingCache,
    #[allow(dead_code)]
    open_files: HashMap<u64, u64>,
    next_fh: u64,
    notification_sender: Option<Sender<Notification>>,
}

impl PassthroughFs {
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
            backing_cache: Default::default(),
            open_files: HashMap::new(),
            next_fh: 0,
            notification_sender: None,
        }
    }

    fn next_fh(&mut self) -> u64 {
        self.next_fh += 1;
        self.next_fh
    }

    // update_backing mutates a BackingStatus held by the backing cache.
    // returns true if the item is valid and should be retained in the cache.
    // returns false if the item is invalid and should be removed from the cache.
    fn update_backing(backing_status: &mut BackingStatus, notifier: Sender<Notification>) -> bool {
        match backing_status {
            BackingStatus::Pending(p) => {
                match p.reply.try_recv() {
                    Ok(Ok(backing_id)) => {
                        log::info!("heartbeat: processing pending {:?}", p);
                        let now = SystemTime::now();
                        *backing_status = BackingStatus::Ready(ReadyBackingId {
                            notifier: notifier.clone(),
                            backing_id,
                            timestamp: now,
                            reply_sender: None,
                        });
                        log::info!("pending -> ready; backing_id {}", backing_id);
                        true
                    }
                    Ok(Err(e)) => {
                        log::error!("heartbeat: processing pending {:?}", p);
                        log::error!("error {}", e);
                        false
                    }
                    Err(crossbeam_channel::TryRecvError::Empty) => {
                        log::debug!("heartbeat: processing pending {:?}", p);
                        log::debug!("waiting for reply");
                        true
                    }
                    Err(crossbeam_channel::TryRecvError::Disconnected) => {
                        log::warn!("heartbeat: processing pending {:?}", p);
                        log::warn!("channel disconnected");
                        false
                    }
                }
            BackingStatus::Ready(r) => {
                let now = SystemTime::now();
                if now.duration_since(r.timestamp).unwrap().as_secs() > 1 {
                    log::info!("heartbeat: processing ready {:?}", r);
                    let (tx, rx) = crossbeam_channel::bounded(1);
                    r.reply_sender = Some(tx);
                    *backing_status = BackingStatus::Dropped(DroppedBackingId { reply: rx });
                    log::info!("ready -> dropped; timeout");
                }
                true // ready transitions to ready or dropped. either way, we keep it in the cache.
            }
            BackingStatus::Dropped(d) => {
                match d.reply.try_recv() {
                    Ok(Ok(value)) => {
                        log::info!("heartbeat: processing dropped {:?}", d);
                        log::info!("ok {:?}", value);
                        false
                    }
                    Ok(Err(e)) => {
                        log::error!("heartbeat: processing dropped {:?}", d);
                        log::error!("error {}", e);
                        false
                    }
                    Err(crossbeam_channel::TryRecvError::Empty) => {
                        log::debug!("heartbeat: processing dropped {:?}", d);
                        log::debug!("waiting for reply");
                        true
                    }
                    Err(crossbeam_channel::TryRecvError::Disconnected) => {
                        log::warn!("heartbeat: processing dropped {:?}", d);
                        log::warn!("channel disconnected");
                        false
                    }
                }
            }
        }
    }
}

impl Filesystem for PassthroughFs {
    fn init(
        &mut self,
        _req: RequestMeta,
        config: KernelConfig,
    ) -> Result<KernelConfig, Errno> {
        let mut config = config;
        config.add_capabilities(consts::FUSE_PASSTHROUGH).unwrap();
        config.set_max_stack_depth(2).unwrap();
        Ok(config)
    }

    #[cfg(feature = "abi-7-11")]
    fn init_notification_sender(
        &mut self,
        sender: Sender<Notification>,
    ) -> bool {
        log::info!("init_notification_sender");
        self.notification_sender = Some(sender);
        true
    }

    fn lookup(&mut self, _req: RequestMeta, parent: u64, name: OsString) -> Result<Entry, Errno> {
        log::info!("lookup(name={:?})", name);
        if parent == 1 && name.to_str() == Some("passthrough") {
            if let std::collections::hash_map::Entry::Vacant(e) =
                self.backing_cache.by_inode.entry(2)
            {
                log::info!("new pending backing id request");
                let (tx, rx) = crossbeam_channel::bounded(1);
                let file = File::open("/etc/os-release").unwrap();
                let fd = std::os::unix::io::AsRawFd::as_raw_fd(&file);
                let backing_id = PendingBackingId {
                    reply: rx,
                    _file: Arc::new(file),
                };
                e.insert(BackingStatus::Pending(backing_id));
                if let Some(sender) = &self.notification_sender {
                    if let Err(e) = sender.send(Notification::OpenBacking((fd as u32, Some(tx)))) {
                        log::error!("failed to send OpenBacking notification: {}", e);
                    }
                } else {
                    log::warn!("unable to request a backing id. no notification sender available");
                }
            }

            Ok(Entry {
                attr: self.passthrough_file_attr,
                ttl: TTL,
                generation: 0,
            })
        } else {
            Err(Errno::ENOENT)
        }
    }

    fn getattr(&mut self,
        _req: RequestMeta,
        ino: u64,
        _fh: Option<u64>,
    ) -> Result<Attr, Errno> {
        match ino {
            1 => Ok(Attr{attr: self.root_attr, ttl: TTL}),
            2 => Ok(Attr{attr: self.passthrough_file_attr, ttl: TTL}),
            _ =>Err(Errno::ENOENT),
        }
    }

    fn open(&mut self, _req: RequestMeta, ino: u64, _flags: i32) -> Result<Open, Errno> {
        if ino != 2 {
            return Err(Errno::ENOENT);
        }

        // let id = self
        //     .backing_cache
        //     .get_or(ino, || {
        //         let _file = File::open("/etc/os-release")?;
        //         // TODO: Implement opening the backing file and returning appropriate
        //         // information, possibly including a BackingId within the Open struct,
        //         // or handle it through other means if fd-passthrough is intended here.
        //         Err(std::io::Error::new(
        //             std::io::ErrorKind::Other,
        //             "TODO: passthrough open not fully implemented",
        //         ))
        //     })
        //     .unwrap();

        let fh = self.next_fh();
        // self.open_files.insert(fh, id.clone());

        // eprintln!("  -> opened_passthrough({fh:?}, 0, {id:?});\n");
        // TODO: Ensure fd-passthrough is correctly set up if intended.
        // The Open struct would carry necessary info.
        // TODO: implement flags for Open struct
        Ok(Open { fh, flags: 0 })
    }

    fn heartbeat(&mut self) -> Result<fuser::FsStatus, Errno> {
        if let Some(notifier) = self.notification_sender.clone() {
            self.backing_cache.by_inode.retain(|_, v| PassthroughFs::update_backing(v, notifier));
        }
        Ok(fuser::FsStatus::Ready)
    }


    fn readdir(
        &mut self,
        _req: RequestMeta,
        ino: u64,
        _fh: u64,
        offset: i64,
        _max_bytes: u32
    ) -> Result<Vec<DirEntry>, Errno> {
        if ino != 1 {
            return Err(Errno::ENOENT);
        }

        let entries = vec![
            (1, FileType::Directory, "."),
            (1, FileType::Directory, ".."),
            (2, FileType::RegularFile, "passthrough"),
        ];
        let mut result = Vec::new();

        for (i, entry) in entries.into_iter().enumerate().skip(offset as usize) {
            // i + 1 means the index of the next entry
            result.push(DirEntry {
                ino: entry.0,
                offset: i as i64 + 1,
                kind: entry.1,
                name: OsString::from(entry.2),
            });
        }
        Ok(result)
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

    let fs = PassthroughFs::new();
    let mut session = fuser::Session::new(fs, &std::path::Path::new(mountpoint), &options).unwrap();
    session.run_with_notifications().unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;
    use std::time::Duration;

    fn dummy_meta() -> RequestMeta {
        RequestMeta {
            unique: 0,
            uid: 1000,
            gid: 1000,
            pid: 2000,
        }
    }

    #[test]
    fn test_lookup_heartbeat_cycle() {
        let mut fs = PassthroughFs::new();
        let (tx, rx) = unbounded();
        fs.init_notification_sender(tx);

        // First lookup should create a pending entry and send a notification
        fs.lookup(dummy_meta(), 1, "passthrough".into()).unwrap();
        assert_eq!(fs.backing_cache.by_inode.len(), 1);
        assert!(matches!(
            fs.backing_cache.by_inode.get(&2).unwrap(),
            BackingStatus::Pending(_)
        ));
        let notification = rx.try_recv().unwrap();
        let (fd, sender) = match notification {
            Notification::OpenBacking(d) => d,
            _ => panic!("unexpected notification"),
        };
        assert!(fd > 0);
        let sender = sender.unwrap();

        // Heartbeat should not do anything yet
        fs.heartbeat().unwrap();
        assert_eq!(fs.backing_cache.by_inode.len(), 1);
        assert!(matches!(
            fs.backing_cache.by_inode.get(&2).unwrap(),
            BackingStatus::Pending(_)
        ));

        // Send a backing id to simulate the kernel
        sender.send(Ok(123)).unwrap();

        // Heartbeat should transition to ready
        fs.heartbeat().unwrap();
        assert_eq!(fs.backing_cache.by_inode.len(), 1);
        assert!(matches!(
            fs.backing_cache.by_inode.get(&2).unwrap(),
            BackingStatus::Ready(_)
        ));

        // Wait for 2 seconds
        std::thread::sleep(Duration::from_secs(2));

        // Heartbeat should remove the ready entry
        fs.heartbeat().unwrap();
        assert_eq!(fs.backing_cache.by_inode.len(), 0);
    }
}
