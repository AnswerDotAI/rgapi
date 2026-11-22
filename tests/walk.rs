use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use rgapi::{FindOptions, StreamIter, WalkOptions, find};

#[test]
fn cancel_and_join_drains_full_channel_and_waits_for_worker() {
    let (tx, rx) = mpsc::sync_channel(1);
    let (started, ready) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let done = Arc::new(AtomicBool::new(false));
    let worker_done = done.clone();
    let worker = thread::spawn(move || {
        tx.send(Ok(1)).unwrap();
        started.send(()).unwrap();
        // This blocks while the channel is full; close must drain, not just join.
        tx.send(Ok(2)).unwrap();
        assert!(worker_cancel.load(Ordering::Acquire));
        worker_done.store(true, Ordering::Release);
    });
    ready.recv().unwrap();
    StreamIter::new(rx, cancel.clone(), Some(worker)).cancel_and_join().unwrap();
    assert!(cancel.load(Ordering::Acquire));
    assert!(done.load(Ordering::Acquire));
}

#[test]
fn globs_match_names_or_root_relative_components() {
    let dir = std::env::temp_dir().join(format!("rgapi-glob-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    for rel in ["src/app.py", "src/app.rs", "src/deep/app.py", "other/src/app.py", "src/tests/app.py"] {
        let full = dir.join(rel);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(&full, "").unwrap();
    }
    let run = |glob: &str, dirs: bool| {
        let opts = FindOptions { walk: WalkOptions { roots: vec![dir.clone()], includes: vec![glob.into()], ignore: false, ..Default::default() }, dirs,
            ..Default::default() };
        find(&opts).unwrap()
    };
    for (glob, dirs, yes, no) in [
        ("*.py", false, "src/deep/app.py", "src/app.rs"), ("src/*", false, "src/app.py", "src/deep/app.py"),
        ("src/**", false, "src/deep/app.py", "other/src/app.py"), ("tests", true, "src/tests", "src/tests/app.py"),
    ] {
        let got = run(glob, dirs);
        assert!(got.contains(&PathBuf::from(yes)), "{glob}: missing {yes} in {got:?}");
        assert!(!got.contains(&PathBuf::from(no)), "{glob}: unexpected {no} in {got:?}");
    }
    let _ = fs::remove_dir_all(&dir);
}

fn iter_of(items: Vec<u32>, delay_ms: u64) -> StreamIter<u32> {
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker = thread::spawn(move || {
        for i in items { thread::sleep(Duration::from_millis(delay_ms)); if tx.send(Ok(i)).is_err() { return; } }
    });
    StreamIter::new(rx, cancel, Some(worker))
}

#[test]
fn collect_timeout_honors_deadline() {
    let (got, timed_out) = iter_of(vec![1, 2, 3], 0).collect_timeout(None).unwrap();
    assert_eq!(got, vec![1, 2, 3]);
    assert!(!timed_out);

    let (got, timed_out) = iter_of(vec![1, 2, 3], 0).collect_timeout(Some(60_000)).unwrap();
    assert_eq!(got, vec![1, 2, 3]);
    assert!(!timed_out);

    let (got, timed_out) = iter_of(vec![1, 2, 3], 50).collect_timeout(Some(0)).unwrap();
    assert!(got.is_empty());
    assert!(timed_out);

    let (got, timed_out) = iter_of(vec![1, 2, 3], 200).collect_timeout(Some(20)).unwrap();
    assert!(got.is_empty());
    assert!(timed_out);
}
