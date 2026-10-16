use super::*;
use async_trait::async_trait;
use belka_core::client::{Sink, Source};
use belka_core::utils::path::{normalize_remote_dir, split_remote};
use belka_core::RemoteFileInfo;

/// A cloud in memory that counts uploads.
#[derive(Default)]
struct FakeCloud {
    files: Mutex<HashMap<String, u64>>,
    uploads: Mutex<Vec<String>>,
}

#[async_trait]
impl StorageProvider for FakeCloud {
    fn name(&self) -> &str {
        "fake"
    }
    async fn ensure_dir(&self, _: &str) -> Result<()> {
        Ok(())
    }
    async fn get_file_info(&self, path: &str) -> Result<Option<RemoteFileInfo>> {
        Ok(self
            .files
            .lock()
            .unwrap()
            .get(path)
            .map(|size| RemoteFileInfo {
                name: split_remote(path).1,
                path: path.to_string(),
                is_dir: false,
                size: *size,
                md5: None,
                modified: Some(Utc::now()),
            }))
    }
    async fn upload(&self, source: &dyn Source, path: &str, _: &Progress) -> Result<()> {
        self.files
            .lock()
            .unwrap()
            .insert(path.to_string(), source.len());
        self.uploads.lock().unwrap().push(path.to_string());
        Ok(())
    }
    async fn download_to(&self, _: &str, _: &mut dyn Sink, _: &Progress) -> Result<u64> {
        unimplemented!("not used by backups")
    }
    async fn delete(&self, _: &str, _: bool) -> Result<()> {
        unimplemented!("backups never delete")
    }
    async fn rename(&self, _: &str, _: &str, _: bool) -> Result<()> {
        unimplemented!("not used by backups")
    }
    async fn list_dir(&self, dir: &str) -> Result<Vec<RemoteFileInfo>> {
        let dir = normalize_remote_dir(dir);
        let paths: Vec<String> = self.files.lock().unwrap().keys().cloned().collect();
        let mut items = Vec::new();
        for path in paths.iter().filter(|p| split_remote(p).0 == dir) {
            items.extend(self.get_file_info(path).await?);
        }
        Ok(items)
    }
}

fn task(local: &Path) -> Task {
    Task {
        name: "Камера".into(),
        ..Task::new(&local.display().to_string(), "a", "/Backup")
    }
}

fn provider(cloud: &Arc<FakeCloud>) -> impl Fn(&str) -> Result<Arc<dyn StorageProvider>> + Sync {
    let cloud: Arc<dyn StorageProvider> = cloud.clone();
    move |_| Ok(cloud.clone())
}

#[tokio::test]
async fn uploads_only_what_is_new_and_keeps_history() {
    let settings = tempfile::tempdir().unwrap();
    let photos = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(photos.path().join("Camera")).unwrap();
    std::fs::write(photos.path().join("Camera/1.jpg"), b"one").unwrap();
    std::fs::write(photos.path().join("Camera/.trashed-2.jpg"), b"two").unwrap();

    let cloud = Arc::new(FakeCloud::default());
    let store = TaskStore::new(settings.path());
    let id = store.save(task(photos.path())).unwrap().id;

    let first = run(&store, &id, &provider(&cloud)).await.unwrap().result;
    assert_eq!((first.uploaded, first.failed), (1, 0));
    assert_eq!(*cloud.uploads.lock().unwrap(), ["/Backup/Camera/1.jpg"]);

    let second = run(&store, &id, &provider(&cloud)).await.unwrap().result;
    assert_eq!((second.uploaded, second.unchanged), (0, 1));

    std::fs::write(photos.path().join("Camera/3.jpg"), b"three").unwrap();
    let third = run(&store, &id, &provider(&cloud)).await.unwrap().result;
    assert_eq!((third.uploaded, third.bytes), (1, 5));

    let saved = store.task(&id).unwrap();
    assert_eq!(saved.history.len(), 3);
    assert_eq!(saved.last().unwrap().uploaded, 1);
    assert!(!is_running(&id));
}

#[tokio::test]
async fn a_moved_task_starts_over() {
    let settings = tempfile::tempdir().unwrap();
    let photos = tempfile::tempdir().unwrap();
    std::fs::write(photos.path().join("1.jpg"), b"one").unwrap();
    let cloud = Arc::new(FakeCloud::default());
    let store = TaskStore::new(settings.path());
    let saved = store.save(task(photos.path())).unwrap();
    run(&store, &saved.id, &provider(&cloud)).await.unwrap();

    // Renaming keeps the history, another target drops it and the index.
    let renamed = store
        .save(Task {
            name: "Фото".into(),
            ..saved.clone()
        })
        .unwrap();
    assert_eq!(renamed.history.len(), 1);
    let moved = store
        .save(Task {
            remote: "/Other".into(),
            ..saved.clone()
        })
        .unwrap();
    assert!(moved.history.is_empty());
    let result = run(&store, &saved.id, &provider(&cloud))
        .await
        .unwrap()
        .result;
    assert_eq!(result.uploaded, 1);
    assert_eq!(
        cloud.uploads.lock().unwrap().last().unwrap(),
        "/Other/1.jpg"
    );
}

#[tokio::test]
async fn frees_space_only_for_old_files_that_are_in_the_cloud() {
    let settings = tempfile::tempdir().unwrap();
    let photos = tempfile::tempdir().unwrap();
    let old = photos.path().join("old.jpg");
    let fresh = photos.path().join("new.jpg");
    std::fs::write(&old, b"old").unwrap();
    std::fs::write(&fresh, b"new").unwrap();
    let month_ago = SystemTime::now() - Duration::from_secs(30 * 86_400);
    std::fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(month_ago)
        .unwrap();

    let cloud = Arc::new(FakeCloud::default());
    let store = TaskStore::new(settings.path());
    let id = store
        .save(Task {
            free_after_days: Some(7),
            ..task(photos.path())
        })
        .unwrap()
        .id;
    let run = run(&store, &id, &provider(&cloud)).await.unwrap();
    assert_eq!((run.result.uploaded, run.result.freed), (2, 1));
    assert_eq!(run.freed, std::slice::from_ref(&old));
    assert!(!old.exists());
    assert!(fresh.exists());
}

#[tokio::test]
async fn two_phones_share_a_folder_without_losing_photos() {
    let cloud = Arc::new(FakeCloud::default());
    let month_ago = SystemTime::now() - Duration::from_secs(30 * 86_400);
    let mut phones = Vec::new();
    for data in [&b"first"[..], b"second"] {
        let settings = tempfile::tempdir().unwrap();
        let photos = tempfile::tempdir().unwrap();
        let photo = photos.path().join("IMG_1.jpg");
        std::fs::write(&photo, data).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&photo)
            .unwrap()
            .set_modified(month_ago)
            .unwrap();
        let store = TaskStore::new(settings.path());
        let id = store
            .save(Task {
                free_after_days: Some(7),
                ..task(photos.path())
            })
            .unwrap()
            .id;
        let run = run(&store, &id, &provider(&cloud)).await.unwrap();
        assert_eq!((run.result.uploaded, run.result.freed), (1, 1));
        phones.push((settings, photos));
    }
    let files = cloud.files.lock().unwrap();
    assert_eq!(files["/Backup/IMG_1.jpg"], 5);
    assert_eq!(files["/Backup/IMG_1 (1).jpg"], 6);
}

#[test]
fn tasks_are_due_after_their_interval() {
    let mut t = Task::new("/x", "a", "/y");
    let now = Utc::now();
    assert!(t.is_due(now));
    t.history.push(RunResult {
        started: Some(now - chrono::Duration::hours(23)),
        finished: Some(now - chrono::Duration::hours(22)),
        ..RunResult::default()
    });
    assert!(!t.is_due(now));
    assert!(t.is_due(now + chrono::Duration::hours(1)));
    t.enabled = false;
    assert!(!t.is_due(now + chrono::Duration::hours(2)));
}

#[tokio::test]
async fn a_killed_run_stays_in_the_history() {
    let settings = tempfile::tempdir().unwrap();
    let photos = tempfile::tempdir().unwrap();
    std::fs::write(photos.path().join("1.jpg"), b"one").unwrap();
    let cloud = Arc::new(FakeCloud::default());
    let store = TaskStore::new(settings.path());
    let id = store.save(task(photos.path())).unwrap().id;
    run(&store, &id, &provider(&cloud)).await.unwrap();

    // A run that started an hour later and never ended, as when the system
    // kills the process.
    let killed = RunResult {
        started: Some(Utc::now() + chrono::Duration::hours(1)),
        ..RunResult::default()
    };
    store.record(&id, &killed).unwrap();
    let saved = store.task(&id).unwrap();
    assert_eq!(saved.history.len(), 2);
    assert!(saved.last().unwrap().finished.is_none());
    // The schedule counts from the run that ended.
    assert!(saved.is_due(Utc::now() + chrono::Duration::hours(24)));

    // The next run is recorded once, with its result.
    run(&store, &id, &provider(&cloud)).await.unwrap();
    let saved = store.task(&id).unwrap();
    assert_eq!(saved.history.len(), 3);
    assert!(saved.last().unwrap().finished.is_some());
}

#[test]
fn saving_cleans_the_task() {
    let settings = tempfile::tempdir().unwrap();
    let store = TaskStore::new(settings.path());
    let saved = store
        .save(Task {
            interval_hours: 0,
            include: vec![" *.jpg ".into(), "".into()],
            free_after_days: Some(0),
            ..Task::new("/storage/emulated/0/DCIM/", "a", "Backup/Camera")
        })
        .unwrap();
    assert_eq!(saved.name, "DCIM");
    assert_eq!(saved.remote, "/Backup/Camera");
    assert_eq!(saved.interval_hours, 1);
    assert_eq!(saved.include, ["*.jpg"]);
    assert_eq!(saved.free_after_days, None);
    assert_eq!(store.find("dcim").unwrap().id, saved.id);
    store.remove(&saved.id).unwrap();
    assert!(store.list().unwrap().is_empty());
}
