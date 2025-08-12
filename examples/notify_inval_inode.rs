// Translated from libfuse's example/notify_{inval_inode,store_retrieve}.c:
//    Copyright (C) 2016 Nikolaus Rath <Nikolaus@rath.org>
//
// Translated to Rust/fuser by Zev Weiss <zev@bewilderbeest.net>
//
// Due to the above provenance, unlike the rest of fuser this file is
// licensed under the terms of the GNU GPLv2.

use std::{
    convert::TryInto,
    ffi::OsStr,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering::SeqCst},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use bytes::Bytes;
use clap::Parser;
use crossbeam_channel::{Receiver, Sender};
use fuser::{
    Dirent, DirentList, Entry, Errno, FUSE_ROOT_ID, FileAttr, FileType, Forget, FsStatus,
    InvalInode, MountOption, Notification, Open, RequestMeta, Store, consts,
    trait_async::Filesystem,
};
use log::{info, warn};

struct ClockFS<'a> {
    file_contents: Arc<Mutex<String>>,
    lookup_cnt: &'a AtomicU64,
    notification_sender: Mutex<Option<Sender<Notification>>>,
    notification_reply: Mutex<Option<Receiver<std::io::Result<()>>>>,
    opts: Arc<Options>,
    last_update: Mutex<SystemTime>,
}

impl ClockFS<'_> {
    const FILE_INO: u64 = 2;
    const FILE_NAME: &'static str = "current_time";

    fn stat(&self, ino: u64) -> Option<FileAttr> {
        let (kind, perm, size) = match ino {
            FUSE_ROOT_ID => (FileType::Directory, 0o755, 0),
            Self::FILE_INO => (
                FileType::RegularFile,
                0o444,
                self.file_contents.lock().unwrap().len(),
            ),
            _ => return None,
        };
        let now = SystemTime::now();
        Some(FileAttr {
            ino,
            size: size.try_into().unwrap(),
            blocks: 0,
            atime: now,
            mtime: now,
            ctime: now,
            crtime: now,
            kind,
            perm,
            nlink: 1,
            uid: 0,
            gid: 0,
            rdev: 0,
            flags: 0,
            blksize: 0,
        })
    }
}

#[async_trait]
impl Filesystem for ClockFS<'_> {
    #[cfg(feature = "abi-7-11")]
    fn init_notification_sender(&mut self, sender: Sender<Notification>) -> bool {
        *self.notification_sender.lock().unwrap() = Some(sender);
        true
    }

    async fn heartbeat(&self) -> FsStatus {
        if let Some(r) = self.notification_reply.lock().unwrap().take() {
            if let Ok(result) = r.try_recv() {
                match result {
                    Ok(()) => info!("Received OK reply"),
                    Err(e) => warn!("Received error reply: {e}"),
                }
            }
        }
        let mut last_update_guard = self.last_update.lock().unwrap();
        let now = SystemTime::now();
        if now.duration_since(*last_update_guard).unwrap_or_default()
            >= Duration::from_secs_f32(self.opts.update_interval)
        {
            let mut s = self.file_contents.lock().unwrap();
            let olddata = std::mem::replace(&mut *s, now_string());
            drop(s); // Release lock on file_contents

            if !self.opts.no_notify && self.lookup_cnt.load(SeqCst) != 0 {
                if let Some(sender) = self.notification_sender.lock().unwrap().as_ref().cloned() {
                    let (s, r) = crossbeam_channel::bounded(1);
                    if self.opts.notify_store {
                        let notification = Notification::Store((
                            Store {
                                ino: Self::FILE_INO,
                                offset: 0,
                                data: self
                                    .file_contents
                                    .lock()
                                    .unwrap()
                                    .as_bytes()
                                    .to_vec()
                                    .into(),
                            },
                            Some(s),
                        ));
                        if let Err(e) = sender.send(notification) {
                            warn!("Warning: failed to send Store notification: {e}");
                        } else {
                            info!("Sent Store notification, preparing for reply.");
                            *self.notification_reply.lock().unwrap() = Some(r);
                        }
                    } else {
                        let notification = Notification::InvalInode((
                            InvalInode {
                                ino: Self::FILE_INO,
                                offset: 0,
                                len: olddata.len().try_into().unwrap_or(-1),
                            },
                            Some(s),
                        ));
                        if let Err(e) = sender.send(notification) {
                            warn!("Warning: failed to send InvalInode notification: {e}");
                        } else {
                            info!("Sent InvalInode notification, preparing for reply.");
                            *self.notification_reply.lock().unwrap() = Some(r);
                        }
                    }
                }
            }
            *last_update_guard = now;
        }
        FsStatus::Ready
    }

    async fn lookup(&self, _req: RequestMeta, parent: u64, name: &Path) -> Result<Entry, Errno> {
        if parent != FUSE_ROOT_ID || name != OsStr::new(Self::FILE_NAME) {
            return Err(Errno::ENOENT);
        }

        self.lookup_cnt.fetch_add(1, SeqCst);
        match self.stat(ClockFS::FILE_INO) {
            Some(attr) => Ok(Entry {
                ino: attr.ino,
                generation: None,
                file_ttl: Duration::MAX,
                attr,
                attr_ttl: Duration::MAX,
            }),
            None => Err(Errno::EIO), // Should not happen
        }
    }

    async fn forget(&self, _req: RequestMeta, target: Forget) {
        if target.ino == ClockFS::FILE_INO {
            let prev = self.lookup_cnt.fetch_sub(target.nlookup, SeqCst);
            assert!(prev >= target.nlookup);
        } else {
            assert!(target.ino == FUSE_ROOT_ID);
        }
    }

    async fn getattr(
        &self,
        _req: RequestMeta,
        ino: u64,
        _fh: Option<u64>,
    ) -> Result<(FileAttr, Duration), Errno> {
        match self.stat(ino) {
            Some(attr) => Ok((attr, Duration::MAX)),
            None => Err(Errno::ENOENT),
        }
    }

    async fn readdir(
        &self,
        _req: RequestMeta,
        ino: u64,
        _fh: u64,
        offset: i64,
        _max_bytes: u32,
    ) -> Result<DirentList, Errno> {
        if ino != FUSE_ROOT_ID {
            return Err(Errno::ENOTDIR);
        }
        // In this example, construct and return an owned vector,
        // containing a reference to borrowed ('static) bytes.
        let mut entries: Vec<Dirent> = Vec::new();
        if offset == 0 {
            let entry_data = Dirent {
                ino: ClockFS::FILE_INO,
                offset: 1, // This entry's cookie
                kind: FileType::RegularFile,
                name: Bytes::from_static(Self::FILE_NAME.as_bytes()),
            };
            entries.push(entry_data);
        }
        // If offset is > 0, we've already returned the single entry during a previous request,
        // so just return the empty vector.
        Ok(entries.into())
    }

    async fn open(&self, _req: RequestMeta, ino: u64, flags: i32) -> Result<Open, Errno> {
        if ino == FUSE_ROOT_ID {
            Err(Errno::EISDIR)
        } else if flags & libc::O_ACCMODE != libc::O_RDONLY {
            Err(Errno::EACCES)
        } else if ino != Self::FILE_INO {
            eprintln!("Got open for nonexistent inode {ino}");
            Err(Errno::ENOENT)
        } else {
            Ok(Open {
                fh: ino, // Using ino as fh, as it's unique for the file
                flags: consts::FOPEN_KEEP_CACHE,
                backing_id: None,
            })
        }
    }

    async fn read(
        &self,
        _req: RequestMeta,
        ino: u64,
        _fh: u64, // fh is ino in this implementation as set in open()
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
    ) -> Result<Bytes, Errno> {
        assert!(ino == Self::FILE_INO);
        if offset < 0 {
            return Err(Errno::EINVAL);
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
        let file_guard = self.file_contents.lock().unwrap();
        let filedata = file_guard.as_bytes();
        let dlen: i64 = filedata.len().try_into().map_err(|_| Errno::EIO)?; // EIO if size doesn't fit i64

        let start_index: usize = offset.try_into().map_err(|_| Errno::EINVAL)?;

        let data_to_return = if start_index > filedata.len() {
            Vec::new() // Read past EOF
        } else {
            let end_index: usize = (offset + i64::from(size))
                .min(dlen) // cap at file length
                .try_into()
                .map_err(|_| Errno::EINVAL)?; // Should not fail if dlen fits usize
            let stop_index = std::cmp::min(end_index, filedata.len());
            filedata[start_index..stop_index].to_vec()
        };

        eprintln!(
            "read returning {} bytes at offset {}",
            data_to_return.len(),
            offset
        );
        Ok(Bytes::from(data_to_return))
    }
}

fn now_string() -> String {
    let Ok(d) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        panic!("Pre-epoch SystemTime");
    };
    format!("The current time is {}\n", d.as_secs())
}

#[derive(Parser, Debug, Clone)]
struct Options {
    /// Mount demo filesystem at given path
    mount_point: String,

    /// Update interval for filesystem contents
    #[clap(short, long, default_value_t = 1.0)]
    update_interval: f32,

    /// Disable kernel notifications
    #[clap(short, long)]
    no_notify: bool,

    /// Use `notify_store()` instead of `notify_inval_inode()`
    #[clap(short = 's', long)]
    notify_store: bool,
}

fn main() {
    let opts = Arc::new(Options::parse());
    let mount_options = vec![
        MountOption::RO,
        MountOption::FSName("clock_inode".to_string()),
    ];
    let fdata = Arc::new(Mutex::new(now_string()));
    let lookup_cnt = Box::leak(Box::new(AtomicU64::new(0))); // Keep as is for simplicity, though not ideal

    env_logger::init();

    let fs = ClockFS {
        file_contents: fdata.clone(),
        lookup_cnt,
        notification_sender: Mutex::new(None), // Will be initialized by the session
        notification_reply: Mutex::new(None),
        opts: opts.clone(),
        last_update: Mutex::new(SystemTime::now()),
    };

    // The main thread will run the FUSE session loop.
    // No separate thread for notification logic is needed anymore.
    // No direct use of session.notifier() from main.
    // No thread::sleep in main's loop.
    // Ctrl-C will be handled by the FUSE session ending, or manually if needed.
    // For now, relying on unmount or external Ctrl-C to stop.

    eprintln!(
        "Mounting ClockFS (inode invalidation) at {}",
        opts.mount_point
    );
    eprintln!("Press Ctrl-C to unmount and exit.");

    // Setup Ctrl-C handler to gracefully unmount (optional but good practice)
    // This part is a bit tricky as Session takes ownership or runs in its own thread.
    // For a direct run like `run_with_notifications`, we might need to handle Ctrl-C
    // to signal the FS to stop or rely on the unmount to terminate.
    // The simplest for now is to let the user unmount the FS to stop.
    // Or, if the session itself handles Ctrl-C, that's also fine.

    let mut session = fuser::Session::new_mounted(fs.into(), &opts.mount_point, &mount_options)
        .expect("Failed to create FUSE session.");

    // Drive the async session loop with a Tokio runtime, matching ioctl.rs style.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(1)
        .build()
        .unwrap();
    session.set_channels(2).unwrap();
    match rt.block_on(session.run_concurrently_parts()) {
        Ok(()) => log::info!("Session ended safely"),
        Err(e) => log::info!("Session ended with error: {e:?}"),
    }

    eprintln!("ClockFS (inode invalidation) unmounted and exited.");
}
