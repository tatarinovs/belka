#[cfg(test)]
mod probes {
    use anyhow::Result;
    use async_trait::async_trait;
    use belka_core::client::{build_http_client, download, PartSink, Sink, Source};
    use belka_core::storage::s3::S3Provider;
    use belka_core::sync::{self, Filter, Index, LocalFile, LocalTree};
    use belka_core::{Progress, RemoteFileInfo, StorageProvider};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, UNIX_EPOCH};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    #[tokio::test]
    async fn long_destination_names_do_not_lengthen_temporary_names() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join(format!("{}.bin", "a".repeat(240)));
        let mut part = PartSink::new(&dest);
        part.write(b"complete").await.unwrap();
        part.commit(&dest).await.unwrap();
        assert_eq!(std::fs::read(dest).unwrap(), b"complete");
    }

    #[tokio::test]
    async fn parallel_sinks_own_different_files_and_preserve_user_part() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out");
        let user = dir.path().join("out.belka.part");
        std::fs::write(&user, b"user").unwrap();
        let (mut a, mut b) = (PartSink::new(&dest), PartSink::new(&dest));
        a.write(b"aaaa").await.unwrap();
        b.write(b"bbbb").await.unwrap();
        a.commit(&dest).await.unwrap();
        b.commit(&dest).await.unwrap();
        assert_eq!(std::fs::read(dest).unwrap(), b"bbbb");
        assert_eq!(std::fs::read(user).unwrap(), b"user");
    }

    #[tokio::test]
    async fn create_only_http_writes_send_preconditions() {
        for kind in ["s3", "webdav"] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = head(&mut socket).await.to_lowercase();
                assert!(request.contains("if-none-match: *"), "{request}");
                let mut body = [0; 1];
                socket.read_exact(&mut body).await.unwrap();
                socket.write_all(b"HTTP/1.1 412 Precondition Failed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
            });
            let provider: Box<dyn StorageProvider> = if kind == "s3" {
                Box::new(
                    S3Provider::new(
                        &endpoint,
                        "bucket".into(),
                        "key".into(),
                        "secret".into(),
                        "us-east-1".into(),
                    )
                    .unwrap(),
                )
            } else {
                Box::new(
                    belka_core::storage::webdav::WebDavProvider::new(
                        "test",
                        &endpoint,
                        "u".into(),
                        "p".into(),
                        false,
                    )
                    .unwrap(),
                )
            };
            let dir = tempfile::tempdir().unwrap();
            let file = dir.path().join("f");
            std::fs::write(&file, b"x").unwrap();
            let err = provider
                .upload_new_file(&file, "/f", &Progress::new())
                .await
                .unwrap_err();
            assert!(
                err.is::<belka_core::storage::AlreadyExists>(),
                "{kind}: {err:#}"
            );
            server.await.unwrap();
        }
    }

    // Regression tests for data loss and interrupted transfers.
    #[test]
    fn dropping_unused_sink_preserves_preexisting_user_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("report.txt");
        let existing = dir.path().join("report.txt.belka.part");
        std::fs::write(&existing, b"user data").unwrap();
        drop(PartSink::new(&dest));
        assert_eq!(std::fs::read(existing).unwrap(), b"user data");
    }

    #[test]
    fn index_detects_same_size_changes_within_one_second() {
        let dir = tempfile::tempdir().unwrap();
        let mut index = Index::load(dir.path().join("index.json"), "target");
        let mut file = LocalFile {
            path: dir.path().join("data"),
            relative: "data".into(),
            size: 4,
            modified: Some(UNIX_EPOCH + Duration::from_millis(100_100)),
        };
        index.record(&file);
        file.modified = Some(UNIX_EPOCH + Duration::from_millis(100_900));
        assert!(
            !index.is_current(&file),
            "changed mtime must be checked again"
        );
    }

    async fn head(socket: &mut TcpStream) -> String {
        let mut buf = Vec::new();
        let mut byte = [0; 1];
        while !buf.ends_with(b"\r\n\r\n") {
            if socket.read(&mut byte).await.unwrap() == 0 {
                break;
            }
            buf.push(byte[0]);
        }
        String::from_utf8(buf).unwrap()
    }

    #[tokio::test]
    async fn resumed_download_restarts_when_etag_changes() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut first, _) = listener.accept().await.unwrap();
            head(&mut first).await;
            first.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nETag: \"v1\"\r\nConnection: close\r\n\r\nAAAA").await.unwrap();
            first.shutdown().await.unwrap();
            drop(first);
            let (mut second, _) = listener.accept().await.unwrap();
            let request = head(&mut second).await.to_lowercase();
            assert!(request.contains("range: bytes=4-"));
            assert!(request.contains("if-range: \"v1\""));
            second.write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Length: 4\r\nContent-Range: bytes 4-7/8\r\nETag: \"v2\"\r\nConnection: close\r\n\r\nDDDD").await.unwrap();
            second.shutdown().await.unwrap();
            drop(second);
            let (mut third, _) = listener.accept().await.unwrap();
            let request = head(&mut third).await.to_lowercase();
            assert!(!request.contains("range: bytes="));
            third.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nETag: \"v2\"\r\nConnection: close\r\n\r\nCCCCDDDD").await.unwrap();
        });
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out");
        let client = build_http_client().unwrap();
        let len = download("probe", &dest, &Progress::new(), || async {
            Ok(client.get(&url))
        })
        .await
        .unwrap();
        server.await.unwrap();
        assert_eq!(len, 8);
        assert_eq!(std::fs::read(dest).unwrap(), b"CCCCDDDD");
    }

    struct TreeCloud {
        deleted: Mutex<Vec<(String, bool)>>,
    }
    #[async_trait]
    impl StorageProvider for TreeCloud {
        fn name(&self) -> &str {
            "review mock"
        }
        async fn ensure_dir(&self, _: &str) -> Result<()> {
            Ok(())
        }
        async fn get_file_info(&self, _: &str) -> Result<Option<RemoteFileInfo>> {
            Ok(None)
        }
        async fn upload(&self, _: &dyn Source, _: &str, _: &Progress) -> Result<()> {
            unreachable!()
        }
        async fn download_to(&self, _: &str, _: &mut dyn Sink, _: &Progress) -> Result<u64> {
            unreachable!()
        }
        async fn delete(&self, path: &str, dir: bool) -> Result<()> {
            self.deleted.lock().unwrap().push((path.into(), dir));
            Ok(())
        }
        async fn rename(&self, _: &str, _: &str, _: bool) -> Result<()> {
            unreachable!()
        }
        async fn list_dir(&self, path: &str) -> Result<Vec<RemoteFileInfo>> {
            Ok(if path == "/Backup" {
                vec![RemoteFileInfo::dir("/Backup/old".into())]
            } else {
                vec![RemoteFileInfo {
                    name: "keep.dat".into(),
                    path: "/Backup/old/keep.dat".into(),
                    is_dir: false,
                    size: 1,
                    md5: None,
                    modified: None,
                }]
            })
        }
    }

    #[tokio::test]
    async fn delete_extra_preserves_parent_of_an_excluded_file() {
        let cloud = TreeCloud {
            deleted: Mutex::new(Vec::new()),
        };
        let filter = Filter::new(&[], &["*.dat".into()], false);
        assert!(!filter.accepts("old/keep.dat", false));
        sync::delete_extra(&cloud, &LocalTree::default(), "/Backup", &filter, false)
            .await
            .unwrap();
        assert!(cloud.deleted.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn s3_rename_keeps_source_after_a_truncated_copy_response() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        let server = tokio::spawn(async move {
            for n in 0..3 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = head(&mut socket).await;
                seen.lock()
                    .unwrap()
                    .push(request.lines().next().unwrap().to_string());
                let response = match n {
                    0 => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
                    1 => {
                        let xml = "<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>";
                        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{xml}", xml.len()).into_bytes()
                    }
                    2 => b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n<Error>".to_vec(),
                    _ => b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n".to_vec(),
                };
                socket.write_all(&response).await.unwrap();
                socket.shutdown().await.unwrap();
            }
        });
        let provider = S3Provider::new(
            &endpoint,
            "bucket".into(),
            "key".into(),
            "secret".into(),
            "us-east-1".into(),
        )
        .unwrap();
        tokio::time::timeout(
            Duration::from_secs(10),
            provider.rename("/source.bin", "dest.bin", false),
        )
        .await
        .unwrap()
        .unwrap_err();
        server.await.unwrap();
        assert!(!requests
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.starts_with("DELETE ")));
    }
}
