use super::*;

#[test]
fn held_sibling_lock_protects_the_rename_window_and_originals() {
    let root = tempfile::tempdir().unwrap();
    let files = Files::new(Some(root.path())).unwrap();
    let mut job = files.start().unwrap();
    fs::write(job.path.join("messages.json"), b"{}").unwrap();
    let result = files.clean(false).unwrap();
    assert_eq!(result["active"], 1);
    assert!(job.path.exists());
    let published = job.publish().unwrap();
    assert!(published.join("messages.json").exists());
    files.clean(false).unwrap();
    assert!(published.join("messages.json").exists());
    assert_eq!(fs::read_dir(files.tmp()).unwrap().count(), 0);
}

#[test]
fn dry_run_is_read_only_and_stale_locks_are_reclaimed() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("not-created");
    let files = Files::new(Some(&data)).unwrap();
    files.clean(true).unwrap();
    assert!(!data.exists());
    safe_mkdir(&files.tmp()).unwrap();
    let id = "qce_00000000000000000000000000000000";
    fs::create_dir(files.tmp().join(id)).unwrap();
    fs::write(files.tmp().join(format!("{id}.lock")), "").unwrap();
    fs::write(files.tmp().join(id).join("part"), "incomplete").unwrap();
    assert_eq!(files.clean(true).unwrap()["candidates"], 1);
    assert!(files.tmp().join(id).join("part").exists());
    assert_eq!(files.clean(false).unwrap()["removed"], 1);
    assert_eq!(fs::read_dir(files.tmp()).unwrap().count(), 0);
}

#[test]
fn publication_never_replaces_an_existing_export() {
    let root = tempfile::tempdir().unwrap();
    let files = Files::new(Some(root.path())).unwrap();
    let mut job = files.start().unwrap();
    let target = files.exports().join(&job.id);
    fs::create_dir(&target).unwrap();
    fs::write(target.join("original"), "keep").unwrap();
    assert!(job.publish().is_err());
    drop(job);
    assert_eq!(fs::read_to_string(target.join("original")).unwrap(), "keep");
    assert_eq!(fs::read_dir(files.tmp()).unwrap().count(), 0);
}

#[test]
fn creation_failure_preserves_existing_directory_and_lock_contention_is_bounded() {
    let root = tempfile::tempdir().unwrap();
    let files = Files::new(Some(root.path())).unwrap();
    safe_mkdir(&files.tmp()).unwrap();
    let id = "qce_00000000000000000000000000000001";
    fs::create_dir(files.tmp().join(id)).unwrap();
    fs::write(files.tmp().join(id).join("original"), "keep").unwrap();
    assert!(files.start_id(id.into()).is_err());
    assert_eq!(
        fs::read_to_string(files.tmp().join(id).join("original")).unwrap(),
        "keep"
    );
    let _guard = files.coordination().unwrap();
    assert_eq!(files.start().err().unwrap().code, "E_RUN_IN_PROGRESS");
}

#[test]
fn concurrent_clean_preserves_a_job_until_publication() {
    let root = tempfile::tempdir().unwrap();
    let files = Files::new(Some(root.path())).unwrap();
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let barrier = &barrier;
        let files = &files;
        let publisher = scope.spawn(move || {
            let mut job = files.start().unwrap();
            fs::write(job.path.join("messages.json"), "{}").unwrap();
            barrier.wait();
            barrier.wait();
            job.publish().unwrap()
        });
        barrier.wait();
        assert_eq!(files.clean(false).unwrap()["active"], 1);
        barrier.wait();
        let path = publisher.join().unwrap();
        files.clean(false).unwrap();
        assert!(path.join("messages.json").exists());
    });
}

#[cfg(unix)]
#[test]
fn explicit_data_root_alias_is_resolved_without_allowing_links_inside_it() {
    use std::os::unix::fs::symlink;
    let parent = tempfile::tempdir().unwrap();
    let actual = tempfile::tempdir().unwrap();
    symlink(actual.path(), parent.path().join("alias")).unwrap();
    let files = Files::new(Some(&parent.path().join("alias/new-data"))).unwrap();
    let job = files.start().unwrap();
    assert!(
        job.path
            .starts_with(fs::canonicalize(actual.path()).unwrap())
    );
}

#[cfg(unix)]
#[test]
fn clean_rejects_linked_ancestors_and_skips_linked_cache_entries() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("original"), "keep").unwrap();
    let files = Files::new(Some(root.path())).unwrap();
    let cache = root.path().join("cache/qce/downloads");
    fs::create_dir_all(&cache).unwrap();
    symlink(outside.path(), cache.join("alias")).unwrap();
    assert_eq!(files.clean(false).unwrap()["skipped"], 1);
    assert!(outside.path().join("original").exists());
    symlink(outside.path(), root.path().join("tmp")).unwrap();
    assert!(files.clean(false).is_err());
    assert!(files.start().is_err());
}
