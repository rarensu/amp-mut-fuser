// No integration tests for non-Linux targets, so turn off the module for now.
#![cfg(target_os = "linux")]

#[test]
#[cfg(all(target_os = "linux", not(feature = "no-rc")))]
fn unmount_no_send() {
    use fuser::{Filesystem, Session};
    use std::rc::Rc;
    use std::thread;
    use std::time::Duration;
    use tempfile::TempDir;

    struct NoSendFS(
        // Rc to make this !Send
        #[allow(dead_code)] Rc<()>,
    );

    impl Filesystem for NoSendFS {}

    let tmpdir: TempDir = tempfile::tempdir().unwrap();
    let mut session = Session::new(NoSendFS(Rc::new(())), tmpdir.path(), &[]).unwrap();
    let mut unmounter = session.unmount_callable();
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(1));
        unmounter.unmount().unwrap();
    });
    session.run().await.unwrap();
}
