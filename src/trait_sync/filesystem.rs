//! FUSE userspace library implementation
//!
//! This is an improved rewrite of the FUSE userspace library (lowlevel interface) to fully take
//! advantage of Rust's architecture. The only thing we rely on in the real libfuse are mount
//! and unmount calls which are needed to establish a fd to talk to the kernel driver.

#![warn(missing_docs, missing_debug_implementations, rust_2018_idioms)]

#[allow(unused_imports)]
use log::{debug, info, warn, error};
/* 
#[cfg(feature = "serializable")]
use serde::de::value::F64Deserializer;
#[cfg(feature = "serializable")]
use serde::{Deserialize, Serialize};
*/
use std::ffi::OsStr;
use std::path::Path;
use std::time::{Duration, SystemTime};
#[allow(clippy::wildcard_imports)] // avoid duplicating feature gates
use crate::ll::{Errno, TimeOrNow};
#[cfg(feature = "abi-7-11")]
use crate::reply::Ioctl;
#[cfg(target_os = "macos")]
use crate::reply::XTimes;
use crate::reply::{Entry, FileAttr, DirentList, Open, Statfs, Xattr, Lock};
#[cfg(feature = "abi-7-21")]
use crate::DirentPlusList;
use crate::{Forget, KernelConfig};
use crate::request::RequestMeta;
use bytes::Bytes;

/// Filesystem trait.
///
/// This trait must be implemented to provide a userspace filesystem via FUSE.
/// These methods correspond to `fuse_lowlevel_ops` in libfuse. Reasonable default
/// implementations are provided here to get a mountable filesystem that does
/// nothing.
#[allow(clippy::too_many_arguments)]
#[allow(unused_variables)] // This is the main API, so variables are named without the underscore even though the defaults may not use them.
#[allow(clippy::missing_errors_doc)] // These default implementations do not define the conditions that cause errors
pub trait Filesystem {
    /// Initialize filesystem.
    /// Called before any other filesystem method.
    /// The kernel module connection can be configured using the `KernelConfig` object.
    /// The method should return `Ok(KernelConfig)` to accept the connection, or `Err(Errno)` to reject it.
    fn init(&mut self, req: RequestMeta, config: KernelConfig) -> Result<KernelConfig, Errno> {
        Ok(config)
    }

    /// Clean up filesystem.
    /// Called on filesystem exit.
    fn destroy(&mut self) {}

    /// Look up a directory entry by name and get its attributes.
    /// The method should return `Ok(Entry)` if the entry is found, or `Err(Errno)` otherwise.
    fn lookup(&mut self, req: RequestMeta, parent: u64, name: &Path) -> Result<Entry, Errno> {
        warn!("[Not Implemented] lookup(parent: {parent:#x?}, name {name:?})");
        Err(Errno::ENOSYS)
    }

    /// Forget about an inode.
    /// The `target.nlookup` parameter indicates the number of lookups previously performed on
    /// this inode. If the filesystem implements inode lifetimes, it is recommended that
    /// inodes acquire a single reference on each lookup, and lose `target.nlookup` references on
    /// each forget. The filesystem may ignore forget calls, if the inodes don't need to
    /// have a limited lifetime. On unmount it is not guaranteed, that all referenced
    /// inodes will receive a forget message. This operation does not return a result.
    fn forget(&mut self, req: RequestMeta, target: Forget) {}

    /// Like forget, but take multiple forget requests at once for performance. The default
    /// implementation will fallback to `forget` for each node. This operation does not return a result.
    #[cfg(feature = "abi-7-16")]
    fn batch_forget(&mut self, req: RequestMeta, nodes: Vec<Forget>) {
        for node in nodes {
            self.forget(req, node);
        }
    }

    /// Get file attributes.
    /// The method should return `Ok(Attr)` with the file attributes, or `Err(Errno)` otherwise.
    fn getattr(&mut self, req: RequestMeta, ino: u64, fh: Option<u64>) -> Result<(FileAttr, Duration), Errno> {
        warn!("[Not Implemented] getattr(ino: {ino:#x?}, fh: {fh:#x?})");
        Err(Errno::ENOSYS)
    }

    /// Set file attributes.
    /// The method should return `Ok(Attr)` with the updated file attributes, or `Err(Errno)` otherwise.
    fn setattr(
        &mut self,
        req: RequestMeta,
        ino: u64,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        atime: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
        ctime: Option<SystemTime>,
        fh: Option<u64>,
        crtime: Option<SystemTime>,
        chgtime: Option<SystemTime>,
        bkuptime: Option<SystemTime>,
        flags: Option<u32>
    ) -> Result<(FileAttr, std::time::Duration), Errno> {
        warn!(
            "[Not Implemented] setattr(ino: {ino:#x?}, mode: {mode:?}, uid: {uid:?}, \
            gid: {gid:?}, size: {size:?}, fh: {fh:?}, flags: {flags:?})"
        );
        Err(Errno::ENOSYS)
    }

    /// Read symbolic link.
    /// The method should return `Ok(Bytes)` with the link target (an OS native string),
    /// or `Err(Errno)` otherwise.
    /// `Bytes` allows for returning data under various ownership models potentially avoiding a copy.
    fn readlink(&mut self, req: RequestMeta, ino: u64) -> Result<Bytes, Errno> {
        warn!("[Not Implemented] readlink(ino: {ino:#x?})");
        Err(Errno::ENOSYS)
    }

    /// Create file node.
    /// Create a regular file, character device, block device, fifo or socket node.
    /// The method should return `Ok(Entry)` with the new entry's attributes, or `Err(Errno)` otherwise.
    fn mknod(
        &mut self,
        req: RequestMeta,
        parent: u64,
        name: &Path,
        mode: u32,
        umask: u32,
        rdev: u32,
    ) -> Result<Entry, Errno> {
        warn!(
            "[Not Implemented] mknod(parent: {parent:#x?}, name: {name:?}, mode: {mode}, \
            umask: {umask:#x?}, rdev: {rdev})"
        );
        Err(Errno::ENOSYS)
    }

    /// Create a directory.
    /// The method should return `Ok(Entry)` with the new directory's attributes, or `Err(Errno)` otherwise.
    fn mkdir(
        &mut self,
        req: RequestMeta,
        parent: u64,
        name: &Path,
        mode: u32,
        umask: u32,
    ) -> Result<Entry, Errno> {
        warn!("[Not Implemented] mkdir(parent: {parent:#x?}, name: {name:?}, mode: {mode}, umask: {umask:#x?})");
        Err(Errno::ENOSYS)
    }

    /// Remove a file.
    /// The method should return `Ok(())` on success, or `Err(Errno)` otherwise.
    fn unlink(&mut self, req: RequestMeta, parent: u64, name: &Path) -> Result<(), Errno> {
        warn!("[Not Implemented] unlink(parent: {parent:#x?}, name: {name:?})");
        Err(Errno::ENOSYS)
    }

    /// Remove a directory.
    /// The method should return `Ok(())` on success, or `Err(Errno)` otherwise.
    fn rmdir(&mut self, req: RequestMeta, parent: u64, name: &Path) -> Result<(), Errno> {
        warn!("[Not Implemented] rmdir(parent: {parent:#x?}, name: {name:?})");
        Err(Errno::ENOSYS)
    }

    /// Create a symbolic link.
    /// The method should return `Ok(Entry)` with the new link's attributes, or `Err(Errno)` otherwise.
    fn symlink(
        &mut self,
        req: RequestMeta,
        parent: u64,
        link_name: &Path,
        target: &Path,
    ) -> Result<Entry, Errno> {
        warn!("[Not Implemented] symlink(parent: {parent:#x?}, link_name: {link_name:?}, target: {target:?})");
        Err(Errno::EPERM) // why isn't this ENOSYS?
    }

    /// Rename a file.
    /// The method should return `Ok(())` on success, or `Err(Errno)` otherwise.
    /// `flags` may be `RENAME_EXCHANGE` or `RENAME_NOREPLACE`.
    fn rename(
        &mut self,
        req: RequestMeta,
        parent: u64,
        name: &Path,
        newparent: u64,
        newname: &Path,
        flags: u32,
    ) -> Result<(), Errno> {
        warn!(
            "[Not Implemented] rename(parent: {parent:#x?}, name: {name:?}, newparent: {newparent:#x?}, \
            newname: {newname:?}, flags: {flags})",
        );
        Err(Errno::ENOSYS)
    }

    /// Create a hard link.
    /// The method should return `Ok(Entry)` with the new link's attributes, or `Err(Errno)` otherwise.
    fn link(
        &mut self,
        req: RequestMeta,
        ino: u64,
        newparent: u64,
        newname: &Path,
    ) -> Result<Entry, Errno> {
        warn!("[Not Implemented] link(ino: {ino:#x?}, newparent: {newparent:#x?}, newname: {newname:?})");
        Err(Errno::EPERM) // why isn't this ENOSYS?
    }

    /// Open a file.
    /// Open flags (with the exception of `O_CREAT`, `O_EXCL`, `O_NOCTTY` and `O_TRUNC`) are
    /// available in `flags`. Filesystem may store an arbitrary file handle (pointer, index,
    /// etc) in `Open.fh`, and use this in other all other file operations (read, write, flush,
    /// release, fsync). Filesystem may also implement stateless file I/O and not store
    /// anything in `Open.fh`. There are also some flags (`direct_io`, `keep_cache`) which the
    /// filesystem may set in `Open.flags`, to change the way the file is opened. See `fuse_file_info`
    /// structure in `<fuse_common.h>` for more details.
    /// The method should return `Ok(Open)` on success, or `Err(Errno)` otherwise.
    fn open(&mut self, req: RequestMeta, ino: u64, flags: i32) -> Result<Open, Errno> {
        warn!("[Not Implemented] open(ino: {ino:#x?}, flags: {flags})");
        Err(Errno::ENOSYS)
    }

    /// Read data.
    /// Read should return exactly the number of bytes requested except on EOF or error,
    /// otherwise the rest of the data will be substituted with zeroes. An exception to
    /// this is when the file has been opened in '`direct_io`' mode, in which case the
    /// return value of the read system call will reflect the return value of this
    /// operation. `fh` will contain the value set by the open method, or will be undefined
    /// if the open method didn't set any value.
    /// The method should return `Ok(Bytes)` with the read data, or `Err(Errno)` otherwise.
    /// `Bytes` allows for returning borrowed or owned data, potentially avoiding data copies.
    ///
    /// `flags`: these are the file flags, such as `O_SYNC`. Only supported with ABI >= 7.9
    /// `lock_owner`: only supported with ABI >= 7.9
    fn read(
        &mut self,
        req: RequestMeta,
        ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        flags: i32,
        lock_owner: Option<u64>,
    ) -> Result<Bytes, Errno> {
        warn!(
            "[Not Implemented] read(ino: {ino:#x?}, fh: {fh}, offset: {offset}, size: {size}, \
            flags: {flags:#x?}, lock_owner: {lock_owner:?})"
        );
        Err(Errno::ENOSYS)
    }

    /// Write data.
    /// Write should return exactly the number of bytes requested except on error. An
    /// exception to this is when the file has been opened in '`direct_io`' mode, in
    /// which case the return value of the write system call will reflect the return
    /// value of this operation. `fh` will contain the value set by the open method, or
    /// will be undefined if the open method didn't set any value.
    /// The method should return `Ok(u32)` with the number of bytes written, or `Err(Errno)` otherwise.
    ///
    /// `write_flags`: will contain `FUSE_WRITE_CACHE`, if this write is from the page cache. If set,
    /// the pid, uid, gid, and fh may not match the value that would have been sent if write caching
    /// is disabled.
    /// `flags`: these are the file flags, such as `O_SYNC`. Only supported with ABI >= 7.9.
    /// `lock_owner`: only supported with ABI >= 7.9.
    fn write(
        &mut self,
        req: RequestMeta,
        ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        write_flags: u32,
        flags: i32,
        lock_owner: Option<u64>,
    ) -> Result<u32, Errno> {
        warn!(
            "[Not Implemented] write(ino: {:#x?}, fh: {}, offset: {}, data.len(): {}, \
            write_flags: {:#x?}, flags: {:#x?}, lock_owner: {:?})",
            ino,
            fh,
            offset,
            data.len(),
            write_flags,
            flags,
            lock_owner
        );
        Err(Errno::ENOSYS)
    }

    /// Flush method.
    /// This is called on each `close()` of the opened file. Since file descriptors can
    /// be duplicated (dup, dup2, fork), for one open call there may be many flush
    /// calls. Filesystems shouldn't assume that flush will always be called after some
    /// writes, or that it will be called at all. `fh` will contain the value set by the
    /// open method, or will be undefined if the open method didn't set any value.
    /// NOTE: the name of the method is misleading, since (unlike fsync) the filesystem
    /// is not forced to flush pending writes. One reason to flush data is if the
    /// filesystem wants to return write errors. If the filesystem supports file locking
    /// operations (setlk, getlk) it should remove all locks belonging to `lock_owner`.
    /// The method should return `Ok(())` on success, or `Err(Errno)` otherwise.
    fn flush(&mut self, req: RequestMeta, ino: u64, fh: u64, lock_owner: u64) -> Result<(), Errno> {
        warn!("[Not Implemented] flush(ino: {ino:#x?}, fh: {fh}, lock_owner: {lock_owner:?})");
        Err(Errno::ENOSYS)
    }

    /// Release an open file.
    /// Release is called when there are no more references to an open file: all file
    /// descriptors are closed and all memory mappings are unmapped. For every open
    /// call there will be exactly one release call. The filesystem may return an
    /// error, but error values are not returned to `close()` or `munmap()` which triggered
    /// the release. `fh` will contain the value set by the open method, or will be undefined
    /// if the open method didn't set any value. `flags` will contain the same flags as for
    /// open.
    /// The method should return `Ok(())` on success, or `Err(Errno)` otherwise.
    fn release(
        &mut self,
        req: RequestMeta,
        ino: u64,
        fh: u64,
        flags: i32,
        lock_owner: Option<u64>,
        flush: bool,
    ) -> Result<(), Errno> {
        Ok(())
    }

    /// Synchronize file contents.
    /// If the `datasync` parameter is non-zero, then only the user data should be flushed,
    /// not the meta data.
    /// The method should return `Ok(())` on success, or `Err(Errno)` otherwise.
    fn fsync(&mut self, req: RequestMeta, ino: u64, fh: u64, datasync: bool) -> Result<(), Errno> {
        warn!("[Not Implemented] fsync(ino: {ino:#x?}, fh: {fh}, datasync: {datasync})");
        Err(Errno::ENOSYS)
    }

    /// Open a directory.
    /// Filesystem may store an arbitrary file handle (pointer, index, etc) in `Open.fh`, and
    /// use this in other all other directory stream operations (readdir, releasedir,
    /// fsyncdir). Filesystem may also implement stateless directory I/O and not store
    /// anything in `Open.fh`, though that makes it impossible to implement standard conforming
    /// directory stream operations in case the contents of the directory can change
    /// between opendir and releasedir.
    /// The method should return `Ok(Open)` on success, or `Err(Errno)` otherwise.
    fn opendir(&mut self, req: RequestMeta, ino: u64, flags: i32) -> Result<Open, Errno> {
        warn!("[Not Implemented] open(ino: {ino:#x?}, flags: {flags})");
        Err(Errno::ENOSYS)
        // TODO: maybe Open{0, 0} instead?
    }

    /// Read directory.
    /// The filesystem should return a list of directory entries.
    /// A buffer will be filled with entries from up to `max_bytes`.
    /// An empty list indicates the end of the stream.
    /// `fh` will contain the value set by the opendir method, or will be undefined if the
    /// opendir method didn't set any value.
    fn readdir(
        &mut self,
        req: RequestMeta,
        ino: u64,
        fh: u64,
        offset: i64,
        max_bytes: u32
    ) -> Result<DirentList, Errno> {
        warn!("[Not Implemented] readdir(ino: {ino:#x?}, fh: {fh}, offset: {offset}, max_bytes: {max_bytes})");
        Err(Errno::ENOSYS)
    }

    /// Read directory.
    /// Similar to `readdir`, but also returns the attributes of each directory entry.
    /// The filesystem should return a list of tuples of directory entries and their attributes.
    /// A buffer will be filled with entries and attributes up to `max_bytes`.
    /// An empty list indicates the end of the stream.
    /// `fh` will contain the value set by the opendir method, or will be
    /// undefined if the opendir method didn't set any value.
    #[cfg(feature = "abi-7-21")]
    fn readdirplus(
        &mut self,
        req: RequestMeta,
        ino: u64,
        fh: u64,
        offset: i64,
        max_bytes: u32,
    ) -> Result<DirentPlusList, Errno> {
        warn!(
            "[Not Implemented] readdirplus(ino: {ino:#x?}, fh: {fh}, \
            offset: {offset}, max_bytes: {max_bytes})"
        );
        Err(Errno::ENOSYS)
    }

    /// Release an open directory.
    /// For every opendir call there will be exactly one releasedir call. `fh` will
    /// contain the value set by the opendir method, or will be undefined if the
    /// opendir method didn't set any value.
    /// The method should return `Ok(())` on success, or `Err(Errno)` otherwise.
    fn releasedir(
        &mut self,
        req: RequestMeta,
        ino: u64,
        fh: u64,
        flags: i32,
    ) -> Result<(), Errno> {
        Ok(())
    }

    /// Synchronize directory contents.
    /// If the `datasync` parameter is set, then only the directory contents should
    /// be flushed, not the meta data. `fh` will contain the value set by the opendir
    /// method, or will be undefined if the opendir method didn't set any value.
    /// The method should return `Ok(())` on success, or `Err(Errno)` otherwise.
    fn fsyncdir(
        &mut self,
        req: RequestMeta,
        ino: u64,
        fh: u64,
        datasync: bool,
    ) -> Result<(), Errno> {
        warn!("[Not Implemented] fsyncdir(ino: {ino:#x?}, fh: {fh}, datasync: {datasync})");
        Err(Errno::ENOSYS)
    }

    /// Get file system statistics.
    /// The method should return `Ok(Statfs)` with the filesystem statistics, or `Err(Errno)` otherwise.
    fn statfs(&mut self, req: RequestMeta, ino: u64) -> Result<Statfs, Errno> {
        warn!("[Not Implemented] statfs(ino: {ino:#x?})");
        Err(Errno::ENOSYS)
        // TODO: Statfs{0, 0, 0, 0, 0, 512, 255, 0}
    }

    /// Set an extended attribute.
    /// The method should return `Ok(())` on success, or `Err(Errno)` otherwise.
    fn setxattr(
        &mut self,
        req: RequestMeta,
        ino: u64,
        name: &OsStr,
        value: &[u8],
        flags: i32,
        position: u32,
    ) -> Result<(), Errno> {
        warn!("[Not Implemented] setxattr(ino: {ino:#x?}, name: {name:?}, flags: {flags:#x?}, position: {position})");
        Err(Errno::ENOSYS)
    }

    /// Get an extended attribute.
    /// If `size` is 0, the size of the value should be returned in `Xattr::Size(u32)`.
    /// If `size` is not 0, and the value fits, the value should be returned in `Xattr::Data(Bytes)`.
    /// `Bytes` allows for returning borrowed or owned data for the attribute value.
    /// If the value does not fit, `Err(Errno::ERANGE)` should be returned.
    /// The method should return `Ok(Xattr)` on success, or `Err(Errno)` otherwise.
    fn getxattr(
        &mut self,
        req: RequestMeta,
        ino: u64,
        name: &OsStr,
        size: u32,
    ) -> Result<Xattr, Errno> {
        warn!("[Not Implemented] getxattr(ino: {ino:#x?}, name: {name:?}, size: {size})");
        Err(Errno::ENOSYS)
    }

    /// List extended attribute names.
    /// If `size` is 0, the size of the names list should be returned in `Xattr::Size(u32)`.
    /// If `size` is not 0, and the names list fits, it should be returned in `Xattr::Data(ByteBox)`.
    /// `ByteBox` allows for returning borrowed or owned data for the concatenated list of names.
    /// If the list does not fit, `Err(Errno::ERANGE)` should be returned.
    /// The method should return `Ok(Xattr)` on success, or `Err(Errno)` otherwise.
    fn listxattr(
        &mut self,
        req: RequestMeta,
        ino: u64,
        size: u32,
    ) -> Result<Xattr, Errno> {
        warn!("[Not Implemented] listxattr(ino: {ino:#x?}, size: {size})");
        Err(Errno::ENOSYS)
    }

    /// Remove an extended attribute.
    /// The method should return `Ok(())` on success, or `Err(Errno)` otherwise.
    fn removexattr(
        &mut self,
        req: RequestMeta,
        ino: u64,
        name: &OsStr,
    ) -> Result<(), Errno> {
        warn!("[Not Implemented] removexattr(ino: {ino:#x?}, name: {name:?})");
        Err(Errno::ENOSYS)
    }

    /// Check file access permissions.
    /// This will be called for the `access()` system call. If the '`default_permissions`'
    /// mount option is given, this method is not called. This method is not called
    /// under Linux kernel versions 2.4.x
    /// The method should return `Ok(())` if access is allowed, or `Err(Errno)` otherwise.
    fn access(&mut self, req: RequestMeta, ino: u64, mask: i32) -> Result<(), Errno> {
        warn!("[Not Implemented] access(ino: {ino:#x?}, mask: {mask})");
        Err(Errno::ENOSYS)
    }

    /// Create and open a file.
    /// If the file does not exist, first create it with the specified mode, and then
    /// open it. You can use any open flags in the `flags` parameter except `O_NOCTTY`.
    /// The filesystem can store any type of file handle (such as a pointer or index)
    /// in `Open.fh`, which can then be used across all subsequent file operations including
    /// read, write, flush, release, and fsync. Additionally, the filesystem may set
    /// certain flags like `direct_io` and `keep_cache` in `Open.flags` to change the way the file is
    /// opened. See `fuse_file_info` structure in `<fuse_common.h>` for more details. If
    /// this method is not implemented or under Linux kernel versions earlier than
    /// 2.6.15, the `mknod()` and `open()` methods will be called instead.
    /// The method should return `Ok((Entry, Open))` with the new entry's attributes and open file information, or `Err(Errno)` otherwise.
    fn create(
        &mut self,
        req: RequestMeta,
        parent: u64,
        name: &Path,
        mode: u32,
        umask: u32,
        flags: i32,
    ) -> Result<(Entry, Open), Errno> {
        warn!(
            "[Not Implemented] create(parent: {parent:#x?}, name: {name:?}, \
            mode: {mode}, umask: {umask:#x?}, flags: {flags:#x?})"
        );
        Err(Errno::ENOSYS)
    }

    /// Test for a POSIX file lock.
    /// The method should return `Ok(Lock)` with the lock information, or `Err(Errno)` otherwise.
    fn getlk(
        &mut self,
        req: RequestMeta,
        ino: u64,
        fh: u64,
        lock_owner: u64,
        start: u64,
        end: u64,
        typ: i32,
        pid: u32,
    ) -> Result<Lock, Errno> {
        warn!(
            "[Not Implemented] getlk(ino: {ino:#x?}, fh: {fh}, lock_owner: {lock_owner}, \
            start: {start}, end: {end}, typ: {typ}, pid: {pid})"
        );
        Err(Errno::ENOSYS)
    }

    /// Acquire, modify or release a POSIX file lock.
    /// For POSIX threads (NPTL) there's a 1-1 relation between `pid` and `owner`, but
    /// otherwise this is not always the case.  For checking lock ownership,
    /// `fi->owner` must be used. The `l_pid` field in `struct flock` should only be
    /// used to fill in this field in `getlk()`. Note: if the locking methods are not
    /// implemented, the kernel will still allow file locking to work locally.
    /// Hence these are only interesting for network filesystems and similar.
    /// The method should return `Ok(())` on success, or `Err(Errno)` otherwise.
    fn setlk(
        &mut self,
        req: RequestMeta,
        ino: u64,
        fh: u64,
        lock_owner: u64,
        start: u64,
        end: u64,
        typ: i32,
        pid: u32,
        sleep: bool,
    ) -> Result<(), Errno> {
        warn!(
            "[Not Implemented] setlk(ino: {ino:#x?}, fh: {fh}, lock_owner: {lock_owner}, \
            start: {start}, end: {end}, typ: {typ}, pid: {pid}, sleep: {sleep})"
        );
        Err(Errno::ENOSYS)
    }

    /// Map block index within file to block index within device.
    /// Note: This makes sense only for block device backed filesystems mounted
    /// with the 'blkdev' option.
    /// The method should return `Ok(u64)` with the device block index, or `Err(Errno)` otherwise.
    fn bmap(&mut self, req: RequestMeta, ino: u64, blocksize: u32, idx: u64) -> Result<u64, Errno> {
        warn!("[Not Implemented] bmap(ino: {ino:#x?}, blocksize: {blocksize}, idx: {idx})");
        Err(Errno::ENOSYS)
    }

    /// Control device.
    /// The method should return `Ok(Ioctl)` with the ioctl result, or `Err(Errno)` otherwise.
    #[cfg(feature = "abi-7-11")]
    fn ioctl(
        &mut self,
        _req: RequestMeta,
        ino: u64,
        fh: u64,
        flags: u32,
        cmd: u32,
        in_data: &[u8],
        out_size: u32,
    ) -> Result<Ioctl, Errno> {
        warn!(
            "[Not Implemented] ioctl(ino: {:#x?}, fh: {}, flags: {}, cmd: {}, \
            in_data.len(): {}, out_size: {})",
            ino,
            fh,
            flags,
            cmd,
            in_data.len(),
            out_size,
        );
        Err(Errno::ENOSYS)
    }

    /// Poll for events.
    /// The method should return `Ok(u32)` with the poll events, or `Err(Errno)` otherwise.
    /// Ok(nonzero) indicates that the file is ready now.
    /// Ok(0) indicates that the file is not ready. In that case,
    /// the filesystem should save the poll handle (`ph`) in its internal structure.
    /// Later events should be sent via the notification channel.
    #[cfg(feature = "abi-7-11")]
    fn poll(
        &mut self,
        req: RequestMeta,
        ino: u64,
        fh: u64,
        ph: u64,
        events: u32,
        flags: u32,
    ) -> Result<u32, Errno> {
        warn!(
            "[Not Implemented] poll(ino: {ino:#x?}, fh: {fh}, \
            ph: {ph:?}, events: {events}, flags: {flags})"
        );
        Err(Errno::ENOSYS)
    }

    /// Preallocate or deallocate space to a file.
    /// The method should return `Ok(())` on success, or `Err(Errno)` otherwise.
    fn fallocate(
        &mut self,
        req: RequestMeta,
        ino: u64,
        fh: u64,
        offset: i64,
        length: i64,
        mode: i32,
    ) -> Result<(), Errno> {
        warn!(
            "[Not Implemented] fallocate(ino: {ino:#x?}, fh: {fh}, offset: {offset}, \
            length: {length}, mode: {mode})"
        );
        Err(Errno::ENOSYS)
    }

    /// Reposition read/write file offset.
    /// The method should return `Ok(i64)` with the new offset, or `Err(Errno)` otherwise.
    #[cfg(feature = "abi-7-24")]
    fn lseek(
        &mut self,
        req: RequestMeta,
        ino: u64,
        fh: u64,
        offset: i64,
        whence: i32,
    ) -> Result<i64, Errno> {
        warn!("[Not Implemented] lseek(ino: {ino:#x?}, fh: {fh}, offset: {offset}, whence: {whence})");
        Err(Errno::ENOSYS)
    }

    /// Copy the specified range from the source inode to the destination inode.
    /// The method should return `Ok(u32)` with the number of bytes copied, or `Err(Errno)` otherwise.
    fn copy_file_range(
        &mut self,
        req: RequestMeta,
        ino_in: u64,
        fh_in: u64,
        offset_in: i64,
        ino_out: u64,
        fh_out: u64,
        offset_out: i64,
        len: u64,
        flags: u32,
    ) -> Result<u32, Errno> {
        warn!(
            "[Not Implemented] copy_file_range(ino_in: {ino_in:#x?}, fh_in: {fh_in}, \
            offset_in: {offset_in}, ino_out: {ino_out:#x?}, fh_out: {fh_out}, \
            offset_out: {offset_out}, len: {len}, flags: {flags})"
        );
        Err(Errno::ENOSYS)
    }

    /// macOS only: Rename the volume. Set `fuse_init_out.flags` during init to
    /// `FUSE_VOL_RENAME` to enable.
    /// The method should return `Ok(())` on success, or `Err(Errno)` otherwise.
    #[cfg(target_os = "macos")]
    fn setvolname(&mut self, req: RequestMeta, name: &OsStr) -> Result<(), Errno> {
        warn!("[Not Implemented] setvolname(name: {name:?})");
        Err(Errno::ENOSYS)
    }

    /// macOS only (undocumented).
    /// The method should return `Ok(())` on success, or `Err(Errno)` otherwise.
    #[cfg(target_os = "macos")]
    fn exchange(
        &mut self,
        req: RequestMeta,
        parent: u64,
        name: &Path,
        newparent: u64,
        newname: &Path,
        options: u64
    ) -> Result<(), Errno> {
        warn!(
            "[Not Implemented] exchange(parent: {parent:#x?}, name: {name:?}, \
            newparent: {newparent:#x?}, newname: {newname:?}, options: {options:?})"
        );
        Err(Errno::ENOSYS)
    }

    /// macOS only: Query extended times (bkuptime and crtime). Set `fuse_init_out.flags`
    /// during init to `FUSE_XTIMES` to enable.
    /// The method should return `Ok(XTimes)` with the extended times, or `Err(Errno)` otherwise.
    #[cfg(target_os = "macos")]
    fn getxtimes(&mut self, req: RequestMeta, ino: u64) -> Result<XTimes, Errno> {
        warn!("[Not Implemented] getxtimes(ino: {ino:#x?})");
        Err(Errno::ENOSYS)
    }
}