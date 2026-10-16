use super::*;
use crate::client::{Sink, Source};
use crate::progress::Progress;
use async_trait::async_trait;
use std::sync::Mutex;

/// A cloud in memory: files with their content, folders implied by paths
/// or created explicitly.
#[derive(Default)]
pub(crate) struct FakeCloud {
    pub files: Mutex<HashMap<String, Vec<u8>>>,
    pub dirs: Mutex<HashSet<String>>,
    /// Paths uploaded, with `true` for `upload_new`.
    pub uploads: Mutex<Vec<(String, bool)>>,
    pub deleted: Mutex<Vec<String>>,
    pub with_md5: bool,
}

impl FakeCloud {
    pub fn put(&self, path: &str, data: &[u8]) {
        self.files
            .lock()
            .unwrap()
            .insert(path.to_string(), data.to_vec());
    }

    fn info(&self, path: &str) -> Option<RemoteFileInfo> {
        if let Some(data) = self.files.lock().unwrap().get(path) {
            return Some(RemoteFileInfo {
                name: split_remote(path).1,
                path: path.to_string(),
                is_dir: false,
                size: data.len() as u64,
                md5: self.with_md5.then(|| {
                    use md5::{Digest, Md5};
                    hex::encode(Md5::digest(data))
                }),
                modified: Some(Utc::now()),
            });
        }
        let prefix = format!("{path}/");
        let implied = self.files.lock().unwrap().keys().any(|p| p.starts_with(&prefix));
        (implied || self.dirs.lock().unwrap().contains(path)).then(|| RemoteFileInfo::dir(path.into()))
    }

    fn store(&self, source: &dyn Source, path: &str, new: bool) {
        self.uploads.lock().unwrap().push((path.to_string(), new));
        self.files
            .lock()
            .unwrap()
            .insert(path.to_string(), vec![0; source.len() as usize]);
    }
}

#[async_trait]
impl StorageProvider for FakeCloud {
    fn name(&self) -> &str {
        "fake"
    }
    async fn ensure_dir(&self, dir: &str) -> Result<()> {
        self.dirs.lock().unwrap().insert(dir.to_string());
        Ok(())
    }
    async fn get_file_info(&self, path: &str) -> Result<Option<RemoteFileInfo>> {
        Ok(self.info(path))
    }
    async fn upload(&self, source: &dyn Source, path: &str, _: &Progress) -> Result<()> {
        self.store(source, path, false);
        Ok(())
    }
    async fn upload_new(&self, source: &dyn Source, path: &str, _: &Progress) -> Result<()> {
        self.store(source, path, true);
        Ok(())
    }
    async fn download_to(&self, path: &str, sink: &mut dyn Sink, _: &Progress) -> Result<u64> {
        let data = self.files.lock().unwrap()[path].clone();
        sink.reset().await?;
        sink.write(&data).await?;
        Ok(data.len() as u64)
    }
    async fn delete(&self, path: &str, is_dir: bool) -> Result<()> {
        self.deleted.lock().unwrap().push(path.to_string());
        let prefix = format!("{path}/");
        self.files
            .lock()
            .unwrap()
            .retain(|p, _| p != path && !(is_dir && p.starts_with(&prefix)));
        self.dirs
            .lock()
            .unwrap()
            .retain(|p| p != path && !p.starts_with(&prefix));
        Ok(())
    }
    async fn rename(&self, _: &str, _: &str, _: bool) -> Result<()> {
        unimplemented!("not used by sync")
    }
    async fn list_dir(&self, dir: &str) -> Result<Vec<RemoteFileInfo>> {
        let prefix = format!("{}/", dir.trim_end_matches('/'));
        let mut names: Vec<String> = self
            .files
            .lock()
            .unwrap()
            .keys()
            .chain(self.dirs.lock().unwrap().iter())
            .filter_map(|p| p.strip_prefix(&prefix))
            .map(|rest| rest.split('/').next().unwrap().to_string())
            .filter(|n| !n.is_empty())
            .collect();
        names.sort();
        names.dedup();
        Ok(names
            .iter()
            .filter_map(|n| self.info(&format!("{prefix}{n}")))
            .collect())
    }
}

fn write(root: &Path, relative: &str, data: &[u8]) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, data).unwrap();
}

fn relatives(tree: &LocalTree) -> Vec<&str> {
    tree.files.iter().map(|f| f.relative.as_str()).collect()
}

#[test]
fn scan_applies_the_filter() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "Camera/a.JPG", b"a");
    write(root, "Camera/.trashed-1-b.jpg", b"b");
    write(root, ".thumbnails/t.jpg", b"t");
    write(root, "Camera/c.tmp", b"c");
    write(root, "cache/x.jpg", b"x");
    write(root, "v.mp4", b"vv");
    std::fs::create_dir_all(root.join("empty")).unwrap();

    let tree = scan(root, &Filter::all()).unwrap();
    assert_eq!(tree.files.len(), 6);
    assert_eq!(tree.dirs, [".thumbnails", "Camera", "cache", "empty"]);

    let filter = Filter::new(
        &["*.jpg".into(), "*.mp4".into()],
        &["cache".into()],
        true,
    );
    let tree = scan(root, &filter).unwrap();
    assert_eq!(relatives(&tree), ["Camera/a.JPG", "v.mp4"]);
    assert_eq!(tree.dirs, ["Camera", "empty"]);

    let single = scan(&root.join("v.mp4"), &Filter::all()).unwrap();
    assert!(single.single_file);
    assert_eq!(relatives(&single), ["v.mp4"]);
}

#[tokio::test]
async fn upload_skips_what_the_cloud_has_and_what_the_index_knows() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("src");
    write(&root, "a.txt", b"aaa");
    write(&root, "sub/b.txt", b"bb");
    let cloud = FakeCloud {
        with_md5: true,
        ..FakeCloud::default()
    };
    cloud.put("/B/a.txt", b"aaa");

    let index = Mutex::new(Index::load(dir.path().join("index.json"), "src->B"));
    let tree = scan(&root, &Filter::all()).unwrap();
    let opts = UploadOptions::default();
    let summary = upload(&cloud, &tree, "/B", Some(&index), &opts, &())
        .await
        .unwrap();
    assert_eq!((summary.transferred, summary.unchanged, summary.failed), (1, 1, 0));
    // Not in the cloud: uploaded without looking the file up again.
    assert_eq!(*cloud.uploads.lock().unwrap(), [("/B/sub/b.txt".to_string(), true)]);
    assert_eq!(index.lock().unwrap().len(), 2);
    index.lock().unwrap().save().unwrap();

    // The second run does not even compare: everything is in the index.
    cloud.files.lock().unwrap().clear();
    let index = Mutex::new(Index::load(dir.path().join("index.json"), "src->B"));
    let summary = upload(&cloud, &tree, "/B", Some(&index), &opts, &())
        .await
        .unwrap();
    assert_eq!((summary.checked, summary.unchanged), (0, 2));

    // An index made for another target is not trusted.
    let other = Index::load(dir.path().join("index.json"), "src->C");
    assert!(other.is_empty());
}

#[tokio::test]
async fn upload_dry_run_and_verify() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.txt", b"aaa");
    std::fs::create_dir_all(dir.path().join("empty")).unwrap();
    let tree = scan(dir.path(), &Filter::all()).unwrap();
    let cloud = FakeCloud::default();

    let dry = UploadOptions {
        dry_run: true,
        create_dirs: true,
        ..UploadOptions::default()
    };
    let summary = upload(&cloud, &tree, "/B", None, &dry, &()).await.unwrap();
    assert_eq!(summary.transferred, 1);
    assert!(cloud.uploads.lock().unwrap().is_empty());

    let real = UploadOptions {
        verify: true,
        create_dirs: true,
        ..UploadOptions::default()
    };
    let summary = upload(&cloud, &tree, "/B", None, &real, &()).await.unwrap();
    assert_eq!((summary.transferred, summary.failed), (1, 0));
    assert!(cloud.dirs.lock().unwrap().contains("/B/empty"));
}

#[tokio::test]
async fn delete_extra_removes_only_what_is_gone_locally() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "keep.txt", b"k");
    write(dir.path(), "sub/keep.txt", b"k");
    let cloud = FakeCloud::default();
    for path in [
        "/B/keep.txt",
        "/B/gone.txt",
        "/B/sub/keep.txt",
        "/B/sub/gone.txt",
        "/B/old/x.txt",
        "/B/notes.tmp",
    ] {
        cloud.put(path, b"x");
    }
    let filter = Filter::new(&[], &["*.tmp".into()], false);
    let tree = scan(dir.path(), &filter).unwrap();

    let dry = delete_extra(&cloud, &tree, "/B", &filter, true).await.unwrap();
    assert_eq!(dry.transferred, 3);
    assert!(cloud.deleted.lock().unwrap().is_empty());

    let summary = delete_extra(&cloud, &tree, "/B", &filter, false)
        .await
        .unwrap();
    assert_eq!(summary.transferred, 3);
    let mut left: Vec<String> = cloud.files.lock().unwrap().keys().cloned().collect();
    left.sort();
    assert_eq!(left, ["/B/keep.txt", "/B/notes.tmp", "/B/sub/keep.txt"]);

    assert!(delete_extra(&cloud, &tree, "/", &filter, false).await.is_err());
    let single = scan(&dir.path().join("keep.txt"), &filter).unwrap();
    assert!(delete_extra(&cloud, &single, "/B", &filter, false).await.is_err());
}

#[tokio::test]
async fn download_restores_a_tree_and_skips_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("restored");
    let cloud = FakeCloud::default();
    cloud.put("/B/a.txt", b"aaa");
    cloud.put("/B/sub/deep/b.txt", b"bb");
    cloud.put("/B/sub/c:d.txt", b"c");

    let opts = DownloadOptions::default();
    let summary = download(&cloud, "/B", &target, &opts).await.unwrap();
    assert_eq!((summary.transferred, summary.failed), (3, 0));
    assert_eq!(std::fs::read(target.join("sub/deep/b.txt")).unwrap(), b"bb");
    assert_eq!(std::fs::read(target.join("sub/c_d.txt")).unwrap(), b"c");

    let summary = download(&cloud, "/B", &target, &opts).await.unwrap();
    assert_eq!((summary.transferred, summary.unchanged), (0, 3));

    let file = download(&cloud, "/B/a.txt", &dir.path().join("one"), &opts)
        .await
        .unwrap();
    assert_eq!(file.transferred, 1);
    assert!(download(&cloud, "/missing", &target, &opts).await.is_err());
}
