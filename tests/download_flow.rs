use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use image::{ImageEncoder, RgbImage, codecs::jpeg::JpegEncoder};
use jmcomic_bot_service::{
    config::Config,
    db::Db,
    jobs::{JobQueue, spawn_workers},
    models::{ArtifactFormat, ArtifactResponse, JobResponse},
    routes::{AppState, router},
    signing::{now_unix, signed_file_url},
    worker_client::WorkerClient,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, query_param},
};

#[tokio::test]
async fn creates_cbz_artifact_from_mock_worker_and_cdn_then_reuses_cache() {
    let upstream = MockServer::start().await;
    let image_bytes = tiny_jpeg(2, 2);
    mount_album(&upstream, "123", json!([]), 1).await;

    Mock::given(method("GET"))
        .and(path("/batch-photo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "photoId": "123",
                "photo": photo_json("123", "Chapter 1", [
                    ("00001.jpg", format!("{}/media/photos/123/00001.jpg", upstream.uri()))
                ])
            }
        ])))
        .expect(1)
        .mount(&upstream)
        .await;

    Mock::given(method("GET"))
        .and(path("/media/photos/123/00001.jpg"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(image_bytes, "image/jpeg"))
        .expect(1)
        .mount(&upstream)
        .await;

    let temp = TempDir::new().unwrap();
    let state = test_state(&upstream, &temp).await;
    spawn_workers(state.queue.clone(), state.clone());
    let app = router(state.clone());

    let created: JobResponse = json_request(
        app.clone(),
        Method::POST,
        "/api/v1/downloads",
        Some(json!({
            "album_id": "123",
            "format": "cbz"
        })),
        Some("test-token"),
    )
    .await;

    let completed = wait_for_completed_job(&state.db, &created.job_id).await;
    assert_eq!(completed.status, "completed");
    let artifact_id = completed.artifact_id.expect("artifact id");
    let artifact = state
        .db
        .get_artifact(&artifact_id)
        .await
        .unwrap()
        .expect("artifact row");
    assert_eq!(artifact.format, "cbz");
    assert_eq!(artifact.page_count, 1);
    assert!(
        tokio::fs::read(artifact.path)
            .await
            .unwrap()
            .starts_with(b"PK")
    );

    let cached: JobResponse = json_request(
        app,
        Method::POST,
        "/api/v1/downloads",
        Some(json!({
            "album_id": "123",
            "format": "cbz"
        })),
        Some("test-token"),
    )
    .await;
    assert_eq!(cached.status, "completed");
    assert!(cached.cached);
    assert_eq!(cached.artifact_id.as_deref(), Some(artifact_id.as_str()));
    assert!(cached.download_url.is_some());
}

#[tokio::test]
async fn health_is_public_and_protected_routes_require_bearer_token() {
    let upstream = MockServer::start().await;
    let temp = TempDir::new().unwrap();
    let app = router(test_state(&upstream, &temp).await);

    let response = request(app.clone(), Method::GET, "/health", None, None).await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = request(app.clone(), Method::GET, "/api/v1/albums/123", None, None).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = request(
        app,
        Method::GET,
        "/api/v1/albums/123",
        None,
        Some("wrong-token"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn search_proxies_worker_defaults_and_reports_bad_requests() {
    let upstream = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .and(query_param("query", "blue"))
        .and(query_param("page", "1"))
        .and(query_param("orderBy", "mr"))
        .and(query_param("time", "a"))
        .and(query_param("mainTag", "0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "total": "1",
            "content": [{"id": "123", "author": "a", "name": "n"}]
        })))
        .expect(1)
        .mount(&upstream)
        .await;

    let temp = TempDir::new().unwrap();
    let app = router(test_state(&upstream, &temp).await);

    let result: Value = json_request(
        app.clone(),
        Method::GET,
        "/api/v1/search?q=blue",
        None,
        Some("test-token"),
    )
    .await;
    assert_eq!(result["total"], "1");
    assert_eq!(result["content"][0]["id"], "123");

    let response = request(app, Method::GET, "/api/v1/search", None, Some("test-token")).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn album_info_sorts_chapters_and_uses_cache() {
    let upstream = MockServer::start().await;
    mount_album(
        &upstream,
        "123",
        json!([
            {"id": "c", "name": "C", "sort": "3"},
            {"id": "a", "name": "A", "sort": "1"},
            {"id": "b", "name": "B", "sort": "2"}
        ]),
        1,
    )
    .await;

    let temp = TempDir::new().unwrap();
    let app = router(test_state(&upstream, &temp).await);

    for _ in 0..2 {
        let result: Value = json_request(
            app.clone(),
            Method::GET,
            "/api/v1/albums/123",
            None,
            Some("test-token"),
        )
        .await;
        let ids = result["chapters"]
            .as_array()
            .unwrap()
            .iter()
            .map(|chapter| chapter["id"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }
}

#[tokio::test]
async fn cover_downloads_processed_jpeg_and_uses_file_cache() {
    let upstream = MockServer::start().await;
    let image_url = format!("{}/media/photos/123/00001.jpg", upstream.uri());
    Mock::given(method("GET"))
        .and(path("/photo/123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(photo_json(
            "123",
            "Chapter 1",
            [("00001.jpg", image_url)],
        )))
        .expect(1)
        .mount(&upstream)
        .await;
    Mock::given(method("GET"))
        .and(path("/media/photos/123/00001.jpg"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(tiny_jpeg(2, 2), "image/jpeg"))
        .expect(1)
        .mount(&upstream)
        .await;

    let temp = TempDir::new().unwrap();
    let app = router(test_state(&upstream, &temp).await);

    for _ in 0..2 {
        let response = request(
            app.clone(),
            Method::GET,
            "/api/v1/albums/123/cover",
            None,
            Some("test-token"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/jpeg"
        );
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        assert!(body.starts_with(&[0xff, 0xd8]));
    }
}

#[tokio::test]
async fn cover_returns_not_found_when_photo_has_no_images() {
    let upstream = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/photo/empty"))
        .respond_with(ResponseTemplate::new(200).set_body_json(photo_json(
            "empty",
            "Empty",
            std::iter::empty::<(&str, String)>(),
        )))
        .mount(&upstream)
        .await;

    let temp = TempDir::new().unwrap();
    let app = router(test_state(&upstream, &temp).await);
    let response = request(
        app,
        Method::GET,
        "/api/v1/albums/empty/cover",
        None,
        Some("test-token"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn downloads_validate_input_and_return_not_found_for_missing_jobs() {
    let upstream = MockServer::start().await;
    let temp = TempDir::new().unwrap();
    let app = router(test_state(&upstream, &temp).await);

    let response = request(
        app.clone(),
        Method::POST,
        "/api/v1/downloads",
        Some(json!({"album_id": " ", "format": "cbz"})),
        Some("test-token"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = request(
        app,
        Method::GET,
        "/api/v1/downloads/missing",
        None,
        Some("test-token"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn artifact_metadata_and_signed_file_download_work_without_bearer() {
    let upstream = MockServer::start().await;
    let temp = TempDir::new().unwrap();
    let state = test_state(&upstream, &temp).await;
    let file_path = temp.path().join("artifact.pdf");
    tokio::fs::write(&file_path, b"%PDF-1.3\n%%EOF")
        .await
        .unwrap();
    state
        .db
        .insert_artifact(
            "artifact-1",
            "hash",
            ArtifactFormat::Pdf,
            "A/B",
            &file_path.to_string_lossy(),
            14,
            "abc123",
            2,
            1,
        )
        .await
        .unwrap();
    let app = router(state);

    let metadata: ArtifactResponse = json_request(
        app.clone(),
        Method::GET,
        "/api/v1/artifacts/artifact-1",
        None,
        Some("test-token"),
    )
    .await;
    assert_eq!(metadata.format, "pdf");
    assert_eq!(metadata.page_count, 2);

    let response = request(app.clone(), Method::GET, &metadata.download_url, None, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/pdf"
    );
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    assert_eq!(&body[..], b"%PDF-1.3\n%%EOF");

    let response = request(
        app,
        Method::GET,
        "/api/v1/files/artifact-1?exp=1&sig=bad",
        None,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn signed_file_reports_expired_or_missing_artifacts() {
    let upstream = MockServer::start().await;
    let temp = TempDir::new().unwrap();
    let state = test_state(&upstream, &temp).await;
    state
        .db
        .insert_artifact(
            "missing-file",
            "hash",
            ArtifactFormat::Zip,
            "Missing",
            &temp.path().join("missing.zip").to_string_lossy(),
            1,
            "abc123",
            1,
            1,
        )
        .await
        .unwrap();
    state
        .db
        .insert_artifact(
            "expired-artifact",
            "hash-expired",
            ArtifactFormat::Zip,
            "Expired",
            &temp.path().join("missing.zip").to_string_lossy(),
            1,
            "abc123",
            1,
            -1,
        )
        .await
        .unwrap();
    let app = router(state);

    let missing_file_url = signed_file_url("test-signing-secret", "missing-file", 60);
    let response = request(app.clone(), Method::GET, &missing_file_url, None, None).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let exp = now_unix() + 60;
    let sig =
        jmcomic_bot_service::signing::sign_artifact("test-signing-secret", "expired-artifact", exp);
    let response = request(
        app,
        Method::GET,
        &format!("/api/v1/files/expired-artifact?exp={exp}&sig={sig}"),
        None,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn artifact_metadata_returns_not_found_for_missing_artifact() {
    let upstream = MockServer::start().await;
    let temp = TempDir::new().unwrap();
    let app = router(test_state(&upstream, &temp).await);
    let response = request(
        app,
        Method::GET,
        "/api/v1/artifacts/missing",
        None,
        Some("test-token"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

async fn test_state(upstream: &MockServer, temp: &TempDir) -> AppState {
    let data_dir = temp.path().join("data");
    let config = Arc::new(Config {
        bot_tokens: vec!["test-token".to_owned()],
        file_signing_secret: "test-signing-secret".to_owned(),
        worker_base_url: upstream.uri(),
        data_dir: data_dir.clone(),
        database_url: format!("sqlite://{}", data_dir.join("test.db").display()),
        max_concurrent_jobs: 1,
        image_concurrency: 2,
        signed_url_ttl_seconds: 60,
        artifact_ttl_days: 1,
        cache_max_bytes: 1_000_000_000,
        max_pages_per_job: 10,
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        jpeg_quality: 90,
        public_base_url: None,
    });
    config.ensure_dirs().await.unwrap();
    let db = Db::connect(&config.database_url).await.unwrap();
    db.init().await.unwrap();

    AppState {
        config: config.clone(),
        db,
        worker: WorkerClient::new(config.worker_base_url.clone()).unwrap(),
        queue: JobQueue::new(1),
    }
}

async fn mount_album(upstream: &MockServer, album_id: &str, series: Value, expected_calls: u64) {
    Mock::given(method("GET"))
        .and(path(format!("/album/{album_id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": album_id,
            "name": "Mock Album",
            "images": ["00001.jpg"],
            "description": "desc",
            "totalViews": "10",
            "likes": "2",
            "series": series,
            "seriesID": "",
            "author": ["author"],
            "tags": ["tag"],
            "works": [],
            "actors": []
        })))
        .expect(expected_calls)
        .mount(upstream)
        .await;
}

async fn request(
    app: Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
    token: Option<&str>,
) -> axum::response::Response {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let body = match body {
        Some(body) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };
    app.oneshot(builder.body(body).unwrap()).await.unwrap()
}

async fn json_request<T>(
    app: Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
    token: Option<&str>,
) -> T
where
    T: serde::de::DeserializeOwned,
{
    let response = request(app, method, uri, body, token).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

async fn wait_for_completed_job(db: &Db, job_id: &str) -> jmcomic_bot_service::db::JobRecord {
    for _ in 0..100 {
        let job = db.get_job(job_id).await.unwrap().expect("job row");
        if job.status == "completed" {
            return job;
        }
        if job.status == "failed" {
            panic!("job failed: {:?}", job.error);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("job did not complete in time");
}

fn photo_json<'a>(
    id: &str,
    name: &str,
    images: impl IntoIterator<Item = (&'a str, String)>,
) -> Value {
    json!({
        "id": id,
        "name": name,
        "images": images.into_iter().map(|(name, url)| json!({
            "name": name,
            "url": url
        })).collect::<Vec<_>>(),
        "scrambleId": 999999
    })
}

fn tiny_jpeg(width: u32, height: u32) -> Vec<u8> {
    let mut image = RgbImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            image.put_pixel(x, y, image::Rgb([x as u8 * 100, y as u8 * 100, 32]));
        }
    }
    let mut bytes = Vec::new();
    let encoder = JpegEncoder::new_with_quality(&mut bytes, 90);
    encoder
        .write_image(
            image.as_raw(),
            width,
            height,
            image::ExtendedColorType::Rgb8,
        )
        .unwrap();
    bytes
}
