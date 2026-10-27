use std::{
    fs::File,
    io::Read,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use axum::{
    Json, Router,
    extract::{Query, Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::{
    db::Database,
    path::{self, windows_file_url, windows_path},
};

#[derive(Clone)]
struct AppState {
    database_path: Arc<PathBuf>,
    wsl_distro: Option<Arc<str>>,
}

#[derive(Debug, Default, Deserialize)]
struct SearchQuery {
    #[serde(default)]
    q: String,
}

#[derive(Debug, Deserialize)]
struct ImageQuery {
    path: String,
}

#[derive(Debug, Deserialize)]
struct OpenRequest {
    path: String,
    application: String,
}

#[derive(Debug, Serialize)]
struct SearchResponse {
    query: String,
    results: Vec<WebSearchResult>,
}

#[derive(Debug, Serialize)]
struct WebSearchResult {
    path: String,
    windows_path: Option<String>,
    windows_url: Option<String>,
    image_url: Option<String>,
    snippet: Option<String>,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

pub async fn serve(database_path: &Path, listen: IpAddr, port: u16) -> anyhow::Result<()> {
    let app = router(database_path.to_owned(), path::detect_wsl_distro());
    let address = SocketAddr::new(listen, port);
    let listener = tokio::net::TcpListener::bind(address).await?;
    println!("Server running on http://{address}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            if let Err(error) = tokio::signal::ctrl_c().await {
                eprintln!("Failed to listen for Ctrl-C: {error}");
            }
            println!("Shutting down...");
        })
        .await?;
    Ok(())
}

fn router(database_path: PathBuf, wsl_distro: Option<String>) -> Router {
    let state = AppState {
        database_path: Arc::new(database_path),
        wsl_distro: wsl_distro.map(Arc::from),
    };
    Router::new()
        .route("/", get(index))
        .route("/app.js", get(javascript))
        .route("/style.css", get(stylesheet))
        .route("/favicon.svg", get(favicon))
        .route("/api/search", get(search))
        .route("/api/open", post(open_with))
        .route("/api/image", get(image))
        .route("/api/archive-image", get(archive_image))
        .with_state(state)
        .layer(middleware::from_fn(access_log))
}

async fn access_log(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let started = Instant::now();
    let response = next.run(request).await;
    eprintln!(
        "{method} {uri} {} {:?}",
        response.status(),
        started.elapsed()
    );
    response
}

async fn index() -> Html<&'static str> {
    Html(include_str!("web/index.html"))
}

async fn javascript() -> ([(header::HeaderName, &'static str); 1], &'static str) {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("web/app.js"),
    )
}

async fn stylesheet() -> ([(header::HeaderName, &'static str); 1], &'static str) {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("web/style.css"),
    )
}

async fn favicon() -> ([(header::HeaderName, &'static str); 1], &'static str) {
    (
        [(header::CONTENT_TYPE, "image/svg+xml")],
        include_str!("web/favicon.svg"),
    )
}

struct CancelSearchOnDrop(Arc<AtomicBool>);

impl Drop for CancelSearchOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

async fn run_search(
    database_path: Arc<PathBuf>,
    query: String,
) -> anyhow::Result<Vec<crate::db::SearchResult>> {
    let cancelled = Arc::new(AtomicBool::new(false));
    let _cancel_on_drop = CancelSearchOnDrop(cancelled.clone());
    tokio::task::spawn_blocking(move || {
        let database = Database::open(&database_path)?;
        if cancelled.load(Ordering::Relaxed) {
            anyhow::bail!("The search was cancelled");
        }
        database.search_cancellable(&query, 100, cancelled)
    })
    .await?
}

async fn search(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<SearchResponse>, (StatusCode, Json<ErrorResponse>)> {
    let query = query.q.trim().to_owned();
    let results = run_search(state.database_path.clone(), query.clone())
        .await
        .map_err(|_| internal_error("The search could not be completed".into()))?;
    let results = results
        .into_iter()
        .map(|result| {
            let path = Path::new(&result.path);
            let windows_path = state
                .wsl_distro
                .as_deref()
                .and_then(|distro| windows_path(path, distro));
            let windows_url = state
                .wsl_distro
                .as_deref()
                .and_then(|distro| windows_file_url(path, distro));
            WebSearchResult {
                image_url: image_url(path),
                path: result.path,
                windows_path,
                windows_url,
                snippet: result.snippet,
            }
        })
        .collect();
    Ok(Json(SearchResponse { query, results }))
}

async fn image(
    State(state): State<AppState>,
    Query(query): Query<ImageQuery>,
) -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
    let path = Path::new(&query.path);
    let content_type = image_content_type(path).ok_or_else(not_found)?;
    ensure_indexed(&state, &query.path)?;
    let bytes =
        std::fs::read(path).map_err(|_| internal_error("The image could not be read".into()))?;
    Ok(([(header::CONTENT_TYPE, content_type)], bytes).into_response())
}

async fn open_with(
    State(state): State<AppState>,
    Json(request): Json<OpenRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    ensure_indexed(&state, &request.path)?;
    let distro = state
        .wsl_distro
        .as_deref()
        .ok_or_else(|| bad_request("Open with is only available from WSL".into()))?;
    let path = windows_path(Path::new(&request.path), distro)
        .ok_or_else(|| bad_request("The path cannot be opened".into()))?;
    application_command(&request.application, &path)?
        .spawn()
        .map_err(|_| internal_error("The application could not be started".into()))?;
    Ok(StatusCode::NO_CONTENT)
}

fn application_command(
    application: &str,
    path: &str,
) -> Result<Command, (StatusCode, Json<ErrorResponse>)> {
    let program = match application {
        "neeview" => "NeeView.exe",
        "zed" => "zed.exe",
        _ => return Err(bad_request("The application is not supported".into())),
    };
    let mut command = Command::new(program);
    command.arg(path);
    Ok(command)
}

async fn archive_image(
    State(state): State<AppState>,
    Query(query): Query<ImageQuery>,
) -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
    let path = Path::new(&query.path);
    if !is_zip(path) {
        return Err(not_found());
    }
    ensure_indexed(&state, &query.path)?;
    let file =
        File::open(path).map_err(|_| internal_error("The archive could not be read".into()))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|_| not_found())?;
    let mut entries = Vec::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|_| not_found())?;
        let name = entry.name().map_err(|_| not_found())?.into_owned();
        entries.push((name, index));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    for (_, index) in entries.into_iter().take(3) {
        let mut entry = archive.by_index(index).map_err(|_| not_found())?;
        if entry.is_dir() {
            continue;
        }
        let entry_name = entry.name().map_err(|_| not_found())?;
        let Some(content_type) = image_content_type(Path::new(entry_name.as_ref())) else {
            continue;
        };
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|_| internal_error("The archived image could not be read".into()))?;
        return Ok(([(header::CONTENT_TYPE, content_type)], bytes).into_response());
    }
    Err(not_found())
}

fn ensure_indexed(state: &AppState, path: &str) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let database = Database::open(&state.database_path)
        .map_err(|_| internal_error("The search database is unavailable".into()))?;
    let indexed = database
        .connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM documents WHERE path = ?1)",
            params![path],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|_| internal_error("The image could not be checked".into()))?;
    if indexed { Ok(()) } else { Err(not_found()) }
}

fn not_found() -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: "The image was not found".into(),
        }),
    )
}

fn bad_request(message: String) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse { error: message }),
    )
}

fn image_url(path: &Path) -> Option<String> {
    let endpoint = if image_content_type(path).is_some() {
        "/api/image"
    } else if is_zip(path) {
        "/api/archive-image"
    } else {
        return None;
    };
    let path = path.to_str()?;
    Some(format!(
        "{endpoint}?path={}",
        utf8_percent_encode(path, NON_ALPHANUMERIC)
    ))
}

fn is_zip(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
}

fn image_content_type(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        "avif" => Some("image/avif"),
        "svg" => Some("image/svg+xml"),
        _ => None,
    }
}

fn internal_error(message: String) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse { error: message }),
    )
}

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::*;

    fn test_database() -> (tempfile::NamedTempFile, Database) {
        let file = tempfile::NamedTempFile::new().unwrap();
        let database = Database::open(file.path()).unwrap();
        (file, database)
    }

    #[tokio::test]
    async fn serves_search_results_as_json() {
        let (file, database) = test_database();
        database.connection.execute(
            "INSERT INTO documents(path, content) VALUES ('/home/user/note.txt', 'a memorable phrase')",
            [],
        ).unwrap();
        drop(database);
        let response = router(file.path().to_owned(), Some("Ubuntu".into()))
            .oneshot(
                Request::get("/api/search?q=memorable")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["results"][0]["path"], "/home/user/note.txt");
        assert_eq!(
            body["results"][0]["windows_path"],
            "\\\\wsl.localhost\\Ubuntu\\home\\user\\note.txt"
        );
    }

    #[tokio::test]
    async fn searches_all_keywords_separated_by_whitespace() {
        let (file, database) = test_database();
        database
            .connection
            .execute_batch(
                "INSERT INTO documents(path, content) VALUES
                    ('/home/user/both.txt', 'alpha text and beta text'),
                    ('/home/user/alpha.txt', 'alpha text only');",
            )
            .unwrap();
        drop(database);
        let response = router(file.path().to_owned(), None)
            .oneshot(
                Request::get("/api/search?q=alpha%20%E3%80%80%20beta")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["results"].as_array().unwrap().len(), 1);
        assert_eq!(body["results"][0]["path"], "/home/user/both.txt");
    }

    #[tokio::test]
    async fn serves_windows_drive_paths_as_json() {
        let (file, database) = test_database();
        database.connection.execute(
            "INSERT INTO documents(path, content) VALUES ('/mnt/c/Users/user/note.txt', 'a memorable phrase')",
            [],
        ).unwrap();
        drop(database);
        let response = router(file.path().to_owned(), Some("Ubuntu".into()))
            .oneshot(
                Request::get("/api/search?q=memorable")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            body["results"][0]["windows_path"],
            "C:\\Users\\user\\note.txt"
        );
        assert_eq!(
            body["results"][0]["windows_url"],
            "file:///C:/Users/user/note.txt"
        );
    }

    #[tokio::test]
    async fn serves_only_indexed_images() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preview.png");
        let bytes = [0x89, b'P', b'N', b'G'];
        std::fs::write(&path, bytes).unwrap();
        let (database_file, database) = test_database();
        database
            .connection
            .execute(
                "INSERT INTO documents(path, content) VALUES (?1, NULL)",
                params![path.to_string_lossy()],
            )
            .unwrap();
        drop(database);
        let app = router(database_file.path().to_owned(), None);
        let response = app
            .clone()
            .oneshot(
                Request::get("/api/search?q=preview.png")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let image_url = body["results"][0]["image_url"].as_str().unwrap();

        let response = app
            .clone()
            .oneshot(Request::get(image_url).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "image/png");
        assert_eq!(
            response.into_body().collect().await.unwrap().to_bytes(),
            bytes.as_slice()
        );

        let response = app
            .oneshot(
                Request::get("/api/image?path=%2Ftmp%2Funindexed.png")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn serves_the_first_image_within_three_archive_entries() {
        fn create_archive(path: &Path, entries: &[(&str, &[u8])]) {
            let file = File::create(path).unwrap();
            let mut archive = zip::ZipWriter::new(file);
            for (name, bytes) in entries {
                archive
                    .start_file(
                        *name,
                        zip::write::SimpleFileOptions::default()
                            .compression_method(zip::CompressionMethod::Deflated),
                    )
                    .unwrap();
                std::io::Write::write_all(&mut archive, bytes).unwrap();
            }
            archive.finish().unwrap();
        }

        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first.zip");
        let second = directory.path().join("second.zip");
        let late = directory.path().join("late.zip");
        let bytes = [0x89, b'P', b'N', b'G'];
        let sorted_bytes = *b"sorted";
        create_archive(&first, &[("preview.png", &bytes), ("later.txt", b"later")]);
        create_archive(
            &second,
            &[
                ("008.png", &bytes),
                ("003.png", &sorted_bytes),
                ("007.png", &bytes),
            ],
        );
        create_archive(
            &late,
            &[
                ("a.txt", b"one"),
                ("b.txt", b"two"),
                ("c.txt", b"three"),
                ("preview.png", &bytes),
            ],
        );
        let (database_file, database) = test_database();
        for path in [&first, &second, &late] {
            database
                .connection
                .execute(
                    "INSERT INTO documents(path, content) VALUES (?1, NULL)",
                    params![path.to_string_lossy()],
                )
                .unwrap();
        }
        drop(database);
        let app = router(database_file.path().to_owned(), None);
        let response = app
            .clone()
            .oneshot(
                Request::get("/api/search?q=first.zip")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let image_url = body["results"][0]["image_url"].as_str().unwrap();
        assert!(image_url.starts_with("/api/archive-image?path="));

        let response = app
            .clone()
            .oneshot(Request::get(image_url).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "image/png");
        assert_eq!(
            response.into_body().collect().await.unwrap().to_bytes(),
            bytes.as_slice()
        );

        let response = app
            .clone()
            .oneshot(
                Request::get(super::image_url(&second).unwrap())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.into_body().collect().await.unwrap().to_bytes(),
            sorted_bytes.as_slice()
        );

        let response = app
            .clone()
            .oneshot(
                Request::get(super::image_url(&late).unwrap())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let response = app
            .oneshot(
                Request::get("/api/archive-image?path=%2Ftmp%2Funindexed.zip")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn dropping_a_search_marks_it_cancelled() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let guard = CancelSearchOnDrop(cancelled.clone());
        assert!(!cancelled.load(Ordering::Relaxed));
        drop(guard);
        assert!(cancelled.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn returns_an_empty_result_for_an_empty_query() {
        let (file, database) = test_database();
        drop(database);
        let response = router(file.path().to_owned(), None)
            .oneshot(Request::get("/api/search?q=").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["results"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn builds_a_neeview_command_with_the_windows_path_as_one_argument() {
        let command =
            application_command("neeview", r"C:\Users\user\My Pictures\book.zip").unwrap();
        assert_eq!(command.get_program(), "NeeView.exe");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [r"C:\Users\user\My Pictures\book.zip"]
        );
    }

    #[test]
    fn builds_a_zed_command_with_the_windows_path_as_one_argument() {
        let command = application_command("zed", r"C:\Users\user\My Project\notes.txt").unwrap();
        assert_eq!(command.get_program(), "zed.exe");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [r"C:\Users\user\My Project\notes.txt"]
        );
    }

    #[tokio::test]
    async fn rejects_an_unknown_open_with_application() {
        let (file, database) = test_database();
        database
            .connection
            .execute(
                "INSERT INTO documents(path, content) VALUES ('/home/user/note.txt', NULL)",
                [],
            )
            .unwrap();
        drop(database);
        let response = router(file.path().to_owned(), Some("Ubuntu".into()))
            .oneshot(
                Request::post("/api/open")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"path":"/home/user/note.txt","application":"unknown"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
