// This example requires fuse 7.40 or later.
//
// To run this example, do the following:
//
//     sudo RUST_LOG=info ./target/debug/examples/passthrough /tmp/mnt &
//     sudo cat /tmp/mnt/passthrough
//     sudo pkill passthrough
//     sudo umount /tmp/mnt

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
const BACKING_TIMEOUT: Duration = Duration::from_secs(2); // 2 seconds

// ----- BackingID -----

use std::io;

#[derive(Debug)]
struct PendingBackingId {
    reply: crossbeam_channel::Receiver<io::Result<u32>>,
    // The file needs to stay open until the kernel finishes processing the openbacking request.
    #[allow(dead_code)]
    _file: Arc<File>,
}

#[derive(Debug)]
struct ReadyBackingId {
    // The notifier is used to safely close the backing id after any miscellaneous unexpected failures.
    notifier: crossbeam_channel::Sender<Notification>,
    backing_id: u32,
    timestamp: SystemTime,
    // The reply_sender is just for extra logging.
    reply_sender: Option<crossbeam_channel::Sender<io::Result<u32>>>,
}

impl Drop for ReadyBackingId {
    fn drop(&mut self) {
        let notification = Notification::CloseBacking((self.backing_id, self.reply_sender.take()));
        let _ = self.notifier.send(notification);
    }
}

// The ClosedBackingId struct is not strictly necessary, but it is used for extra logging.
#[derive(Debug)]
struct ClosedBackingId {
    reply: crossbeam_channel::Receiver<io::Result<u32>>,
}

#[derive(Debug)]
enum BackingStatus {
    Pending(PendingBackingId),
    Ready(ReadyBackingId),
    Closed(ClosedBackingId),
}

use crossbeam_channel::Sender;

#[derive(Debug)]
struct PassthroughFs {
    root_attr: FileAttr,
    passthrough_file_attr: FileAttr,
    backing_cache: HashMap<u64, BackingStatus>,
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
            backing_cache: HashMap::new(),
            next_fh: 0,
            notification_sender: None,
        }
    }

    fn next_fh(&mut self) -> u64 {
        self.next_fh += 1;
        self.next_fh
    }

    // update_backing mutates a BackingStatus held by the backing cache.
    // It returns a boolean indicating whether the item is still valid and should be retained in the cache.
    // This is so it works with `HashMap::retain` for efficiently dropping stale cache entries.
    fn update_backing_status(
        backing_status: &mut BackingStatus,
        notifier: &Sender<Notification>,
        extend: bool,
    ) -> bool {
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
            }
            BackingStatus::Ready(r) => {
                let now = SystemTime::now();
                if extend {
                    r.timestamp = now;
                } else if now.duration_since(r.timestamp).unwrap() > BACKING_TIMEOUT {
                    log::info!("heartbeat: processing ready {:?}", r);
                    let (tx, rx) = crossbeam_channel::bounded(1);
                    r.reply_sender = Some(tx);
                    *backing_status = BackingStatus::Closed(ClosedBackingId { reply: rx });
                    log::info!("ready -> closed; timeout");
                }
                true // ready transitions to ready or closed. either way, we keep it in the cache.
            }
            BackingStatus::Closed(d) => {
                match d.reply.try_recv() {
                    Ok(Ok(value)) => {
                        log::info!("heartbeat: processing closed {:?}", d);
                        log::info!("ok {:?}", value);
                        false
                    }
                    Ok(Err(e)) => {
                        log::error!("heartbeat: processing closed {:?}", d);
                        log::error!("error {}", e);
                        false
                    }
                    Err(crossbeam_channel::TryRecvError::Empty) => {
                        log::debug!("heartbeat: processing closed {:?}", d);
                        log::debug!("waiting for reply");
                        true
                    }
                    Err(crossbeam_channel::TryRecvError::Disconnected) => {
                        log::warn!("heartbeat: processing closed {:?}", d);
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

    // It is not safe to ioctl the fuse connection to obtain a backing id while the kernel is waiting for your response to an open operation you just accepted.
    // Therefore, we must get the backing id on lookup instead of during open.
    fn lookup(&mut self, _req: RequestMeta, parent: u64, name: OsString) -> Result<Entry, Errno> {
        log::info!("lookup(name={:?})", name);
        if parent == 1 && name.to_str() == Some("passthrough") {
            let mut remove = false;
            if let Some(backing_status) = self.backing_cache.get_mut(&2) {
                if let Some(notifier) = self.notification_sender.clone() {
                    if !Self::update_backing_status(backing_status, &notifier, true) {
                        remove = true;
                    }
                }
            }
            // Mutation is weird. It is not safe to mutate the hash map while we have a mutable reference to one of its values.
            // Therefore, we must drop the mutable reference before we can remove the item from the hash map.
            if remove {
                self.backing_cache.remove(&2);
            }

            if self.backing_cache.get(&2).is_none() {
                log::info!("new pending backing id request");
                if let Some(sender) = &self.notification_sender {
                    let (tx, rx) = crossbeam_channel::bounded(1);
                    let file = File::open("/etc/os-release").unwrap();
                    let fd = std::os::unix::io::AsRawFd::as_raw_fd(&file);
                    if let Err(e) = sender.send(Notification::OpenBacking((fd as u32, Some(tx)))) {
                        log::error!("failed to send OpenBacking notification: {}", e);
                    } else {
                        let backing_id = PendingBackingId {
                            reply: rx,
                            _file: Arc::new(file),
                        };
                        self.backing_cache
                            .insert(2, BackingStatus::Pending(backing_id));
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

        let mut remove = false;
        let mut backing_id_option: Option<u32> = None;
        if let Some(backing_status) = self.backing_cache.get_mut(&ino) {
            if let Some(notifier) = self.notification_sender.clone() {
                if !Self::update_backing_status(backing_status, &notifier, true) {
                    remove = true;
                }
            }
            if let BackingStatus::Ready(ready_backing_id) = backing_status {
                backing_id_option = Some(ready_backing_id.backing_id);
            }
        }
        if remove {
            self.backing_cache.remove(&ino);
        }
        let fh = self.next_fh();
        // TODO: track file handles
        log::info!("open: fh {}", fh);
        Ok(Open {
            fh,
            flags: consts::FOPEN_PASSTHROUGH,
            backing_id: backing_id_option,
        })
    }

    // The heartbeat function is called periodically by the FUSE session.
    // We use it to ensure that the cache entries have accurate timestamps.
    fn heartbeat(&mut self) -> Result<fuser::FsStatus, Errno> {
        if let Some(notifier) = self.notification_sender.clone() {
            self.backing_cache
                .retain(|_, v| PassthroughFs::update_backing_status(v, &notifier, false));
        }
        Ok(fuser::FsStatus::Ready)
    }

    // This deliberately unimplemented read() function is used to prove that the example demonstrates passthrough.
    fn read(
        &mut self,
        _req: RequestMeta,
        _ino: u64,
        _fh: u64,
        _offset: i64,
        _size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
    ) -> Result<Vec<u8>, Errno> {
        unimplemented!();
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
    // A flush() implementation would be a nice addition to the example.
    // The kernel seems to want to perform that operation after a passthrough open().
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
    log::info!("unmounted");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;

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
        assert_eq!(fs.backing_cache.len(), 1);
        assert!(matches!(
            fs.backing_cache.get(&2).unwrap(),
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
        assert_eq!(fs.backing_cache.len(), 1);
        assert!(matches!(
            fs.backing_cache.get(&2).unwrap(),
            BackingStatus::Pending(_)
        ));

        // Send a backing id to simulate the kernel
        sender.send(Ok(123)).unwrap();

        // Heartbeat should transition to ready
        fs.heartbeat().unwrap();
        assert_eq!(fs.backing_cache.len(), 1);
        assert!(matches!(
            fs.backing_cache.get(&2).unwrap(),
            BackingStatus::Ready(_)
        ));

        // Open the file
        let open = fs.open(dummy_meta(), 2, 0).unwrap();
        assert_eq!(open.flags, consts::FOPEN_PASSTHROUGH);
        assert_eq!(open.backing_id, Some(123));

        // Wait for timeout
        std::thread::sleep(BACKING_TIMEOUT);

        // Heartbeat should transition to closed
        fs.heartbeat().unwrap();
        assert_eq!(fs.backing_cache.len(), 1);
        assert!(matches!(
            fs.backing_cache.get(&2).unwrap(),
            BackingStatus::Closed(_)
        ));

        // Simulate kernel reply
        let notification = rx.try_recv().unwrap();
        let (backing_id, sender) = match notification {
            Notification::CloseBacking(d) => d,
            _ => panic!("unexpected notification"),
        };
        assert_eq!(backing_id, 123);
        sender.unwrap().send(Ok(0)).unwrap();

        // Heartbeat should remove the closed entry
        fs.heartbeat().unwrap();
        assert_eq!(fs.backing_cache.len(), 0);
    }
}
