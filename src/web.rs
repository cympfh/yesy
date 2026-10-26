use std::{
    net::{IpAddr, SocketAddr},
    path::Path,
    sync::{Arc, Mutex},
};

use axum::{
    Json, Router,
    extract::{Query, State},
    http::{StatusCode, header},
    response::Html,
    routing::get,
};
use serde::{Deserialize, Serialize};

use crate::{
    db::Database,
    path::{self, windows_file_url, windows_path},
};

#[derive(Clone)]
struct AppState {
    database: Arc<Mutex<Database>>,
    wsl_distro: Option<Arc<str>>,
}

#[derive(Debug, Default, Deserialize)]
struct SearchQuery {
    #[serde(default)]
    q: String,
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
    snippet: Option<String>,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

pub async fn serve(database_path: &Path, listen: IpAddr, port: u16) -> anyhow::Result<()> {
    let database = Database::open(database_path)?;
    let app = router(database, path::detect_wsl_distro());
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

fn router(database: Database, wsl_distro: Option<String>) -> Router {
    let state = AppState {
        database: Arc::new(Mutex::new(database)),
        wsl_distro: wsl_distro.map(Arc::from),
    };
    Router::new()
        .route("/", get(index))
        .route("/app.js", get(javascript))
        .route("/style.css", get(stylesheet))
        .route("/api/search", get(search))
        .with_state(state)
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

async fn search(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<SearchResponse>, (StatusCode, Json<ErrorResponse>)> {
    let query = query.q.trim().to_owned();
    let results = state
        .database
        .lock()
        .map_err(|_| internal_error("The search database is unavailable".into()))?
        .search(&query, 100)
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
                path: result.path,
                windows_path,
                windows_url,
                snippet: result.snippet,
            }
        })
        .collect();
    Ok(Json(SearchResponse { query, results }))
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

    #[tokio::test]
    async fn serves_search_results_as_json() {
        let database = Database::open_in_memory().unwrap();
        database.connection.execute(
            "INSERT INTO documents(path, content) VALUES ('/home/user/note.txt', 'a memorable phrase')",
            [],
        ).unwrap();
        let response = router(database, Some("Ubuntu".into()))
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
    async fn returns_an_empty_result_for_an_empty_query() {
        let database = Database::open_in_memory().unwrap();
        let response = router(database, None)
            .oneshot(Request::get("/api/search?q=").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["results"].as_array().unwrap().len(), 0);
    }
}
