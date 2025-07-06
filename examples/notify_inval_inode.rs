// Translated from libfuse's example/notify_{inval_inode,store_retrieve}.c:
//    Copyright (C) 2016 Nikolaus Rath <Nikolaus@rath.org>
//
// Translated to Rust/fuser by Zev Weiss <zev@bewilderbeest.net>
//
// Due to the above provenance, unlike the rest of fuser this file is
// licensed under the terms of the GNU GPLv2.

use std::{
    convert::TryInto,
    ffi::{OsStr, OsString},
    sync::{
        atomic::{AtomicU64, Ordering::SeqCst},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use clap::Parser;
use crossbeam_channel::Sender;
use fuser::{
    consts, Attr, DirEntry, Entry, Errno, FileAttr, FileType, Filesystem, Forget, FsStatus,
    InvalInode, MountOption, Notification, Open, RequestMeta, Store, FUSE_ROOT_ID,
};

struct ClockFS<'a> {
    file_contents: Arc<Mutex<String>>,
    lookup_cnt: &'a AtomicU64,
    notification_sender: Option<Sender<Notification>>,
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

impl Filesystem for ClockFS<'_> {
    fn init_notification_sender(
        &mut self,
        sender: Sender<Notification>,
    ) -> bool {
        self.notification_sender = Some(sender);
        true
    }

    fn heartbeat(&mut self) -> Result<FsStatus, Errno> {
        let mut last_update_guard = self.last_update.lock().unwrap();
        let now = SystemTime::now();
        if now.duration_since(*last_update_guard).unwrap_or_default() >= Duration::from_secs_f32(self.opts.update_interval) {
            let mut s = self.file_contents.lock().unwrap();
            let olddata = std::mem::replace(&mut *s, now_string());
            drop(s); // Release lock on file_contents

            if !self.opts.no_notify && self.lookup_cnt.load(SeqCst) != 0 {
                if let Some(sender) = &self.notification_sender {
                    let notification = if self.opts.notify_store {
                        Notification::Store(Store {
                            ino: Self::FILE_INO,
                            offset: 0,
                            data: self.file_contents.lock().unwrap().as_bytes().to_vec(),
                        })
                    } else {
                        Notification::InvalInode(InvalInode {
                            ino: Self::FILE_INO,
                            offset: 0,
                            len: olddata.len().try_into().unwrap_or(-1),
                        })
                    };
                    if let Err(e) = sender.send(notification) {
                        eprintln!("Warning: failed to send notification: {}", e);
                    }
                }
            }
            *last_update_guard = now;
        }
        Ok(FsStatus::Ready)
    }

    fn lookup(&mut self, _req: RequestMeta, parent: u64, name: OsString) -> Result<Entry, Errno> {
        if parent != FUSE_ROOT_ID || name != OsStr::new(Self::FILE_NAME) {
            return Err(Errno::ENOENT);
        }

        self.lookup_cnt.fetch_add(1, SeqCst);
        match self.stat(ClockFS::FILE_INO) {
            Some(attr) => Ok(Entry {
                attr,
                ttl: Duration::MAX, // Effectively infinite TTL
                generation: 0,
            }),
            None => Err(Errno::EIO), // Should not happen
        }
    }

    fn forget(&mut self, _req: RequestMeta, target: Forget) {
        if target.ino == ClockFS::FILE_INO {
            let prev = self.lookup_cnt.fetch_sub(target.nlookup, SeqCst);
            assert!(prev >= target.nlookup);
        } else {
            assert!(target.ino == FUSE_ROOT_ID);
        }
    }

    fn getattr(&mut self, _req: RequestMeta, ino: u64, _fh: Option<u64>) -> Result<Attr, Errno> {
        match self.stat(ino) {
            Some(attr) => Ok(Attr {
                attr,
                ttl: Duration::MAX, // Effectively infinite TTL
            }),
            None => Err(Errno::ENOENT),
        }
    }

    fn readdir(
        &mut self,
        _req: RequestMeta,
        ino: u64,
        _fh: u64,
        offset: i64,
        _max_bytes: u32,
    ) -> Result<Vec<DirEntry>, Errno> {
        if ino != FUSE_ROOT_ID {
            return Err(Errno::ENOTDIR);
        }
        let mut entries = Vec::new();
        if offset == 0 {
            entries.push(DirEntry {
                ino: ClockFS::FILE_INO,
                offset: 1, // Next offset
                kind: FileType::RegularFile,
                name: OsString::from(Self::FILE_NAME),
            });
        }
        // If offset is > 0, we've already returned the single entry, so return an empty vector.
        Ok(entries)
    }

    fn open(&mut self, _req: RequestMeta, ino: u64, flags: i32) -> Result<Open, Errno> {
        if ino == FUSE_ROOT_ID {
            Err(Errno::EISDIR)
        } else if flags & libc::O_ACCMODE != libc::O_RDONLY {
            Err(Errno::EACCES)
        } else if ino != Self::FILE_INO {
            eprintln!("Got open for nonexistent inode {}", ino);
            Err(Errno::ENOENT)
        } else {
            Ok(Open {
                fh: ino, // Using ino as fh, as it's unique for the file
                flags: consts::FOPEN_KEEP_CACHE,
            })
        }
    }

    fn read(
        &mut self,
        _req: RequestMeta,
        ino: u64,
        _fh: u64, // fh is ino in this implementation as set in open()
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
    ) -> Result<Vec<u8>, Errno> {
        assert!(ino == Self::FILE_INO);
        if offset < 0 {
            return Err(Errno::EINVAL);
        }
        let file = self.file_contents.lock().unwrap();
        let filedata = file.as_bytes();
        let dlen: i64 = filedata.len().try_into().map_err(|_| Errno::EIO)?; // EIO if size doesn't fit i64

        let start_offset: usize = offset.try_into().map_err(|_| Errno::EINVAL)?;
        if start_offset > filedata.len() {
            return Ok(Vec::new()); // Read past EOF
        }

        let end_offset: usize = (offset + i64::from(size))
            .min(dlen) // cap at file length
            .try_into()
            .map_err(|_| Errno::EINVAL)?; // Should not fail if dlen fits usize

        let actual_end = std::cmp::min(end_offset, filedata.len());

        eprintln!(
            "read returning {} bytes at offset {}",
            actual_end.saturating_sub(start_offset),
            offset
        );
        Ok(filedata[start_offset..actual_end].to_vec())
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

    /// Use notify_store() instead of notify_inval_inode()
    #[clap(short = 's', long)]
    notify_store: bool,
}

fn main() {
    let opts = Arc::new(Options::parse());
    let mount_options = vec![MountOption::RO, MountOption::FSName("clock_inode".to_string())];
    let fdata = Arc::new(Mutex::new(now_string()));
    let lookup_cnt = Box::leak(Box::new(AtomicU64::new(0))); // Keep as is for simplicity, though not ideal

    let fs = ClockFS {
        file_contents: fdata.clone(),
        lookup_cnt,
        notification_sender: None, // Will be initialized by the session
        opts: opts.clone(),
        last_update: Mutex::new(SystemTime::now()),
    };

    // The main thread will run the FUSE session loop.
    // No separate thread for notification logic is needed anymore.
    // No direct use of session.notifier() from main.
    // No thread::sleep in main's loop.
    // Ctrl-C will be handled by the FUSE session ending, or manually if needed.
    // For now, relying on unmount or external Ctrl-C to stop.

    eprintln!("Mounting ClockFS (inode invalidation) at {}", opts.mount_point);
    eprintln!("Press Ctrl-C to unmount and exit.");

    // Setup Ctrl-C handler to gracefully unmount (optional but good practice)
    // This part is a bit tricky as Session takes ownership or runs in its own thread.
    // For a direct run like `run_with_notifications`, we might need to handle Ctrl-C
    // to signal the FS to stop or rely on the unmount to terminate.
    // The simplest for now is to let the user unmount the FS to stop.
    // Or, if the session itself handles Ctrl-C, that's also fine.

    let mut session = fuser::Session::new(fs, &opts.mount_point, &mount_options)
        .unwrap_or_else(|e| panic!("Failed to create FUSE session: {}", e));

    session
        .run_with_notifications()
        .expect("Failed to run FUSE session with notifications");

    eprintln!("ClockFS (inode invalidation) unmounted and exited.");
}
