mod parser;

use axum::body::Body;
use axum::{
    Router,
    extract::{Request, State},
    http::StatusCode,
    response::Response,
};
use parser::Config;
use std::sync::Arc;
use tower_http::trace::TraceLayer;

struct AppState {
    config: Config,
    http_client: reqwest::Client,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let config = parser::parse_config_from_file("./config.json").unwrap();

    let app = app_router(config);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();

    tracing::info!("Gateway listening on {}", listener.local_addr().unwrap());

    axum::serve(listener, app).await.unwrap();
}

fn app_router(config: Config) -> Router {
    let state = Arc::new(AppState {
        config,
        http_client: reqwest::Client::new(),
    });

    Router::new()
        .layer(TraceLayer::new_for_http())
        .fallback(proxy_handler)
        .with_state(state)
}

async fn proxy_handler(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
) -> Result<Response<Body>, StatusCode> {
    let path = req.uri().path();
    let query = req.uri().query().unwrap_or_default();

    let endpoint = state
        .config
        .endpoints
        .iter()
        .find(|e| path.starts_with(&e.path));
    let Some(endpoint) = endpoint else {
        tracing::warn!("No routing config found for path: {}", path);
        return Err(StatusCode::NOT_FOUND);
    };

    let downstream_url = if query.is_empty() {
        format!("{}{}", endpoint.base, path)
    } else {
        format!("{}{}?{}", endpoint.base, path, query)
    };
    tracing::info!("Proxying request to: {}", downstream_url);

    let method = req.method().clone();
    let mut headers = req.headers().clone();

    headers.remove(axum::http::header::HOST);

    let body_bytes = axum::body::to_bytes(req.into_body(), usize::MAX)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    let res = state
        .http_client
        .request(method, downstream_url)
        .headers(headers)
        .body(body_bytes)
        .send()
        .await
        .map_err(|e| {
            tracing::error!("Upstream request failed: {}", e);
            StatusCode::BAD_GATEWAY
        })?;

    let mut response_builder = axum::http::Response::builder().status(res.status());
    if let Some(headers_mut) = response_builder.headers_mut() {
        for (k, v) in res.headers() {
            headers_mut.insert(k, v.clone());
        }
    }

    let res_bytes = res
        .bytes()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(response_builder.body(Body::from(res_bytes)).unwrap())
}

// ==========================================
// TESTS
// ==========================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_config_from_str;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_not_found_endpoint() {
        let raw_config = r#"{ "endpoints": [] }"#;
        let config = parse_config_from_str(raw_config).unwrap();
        let app = app_router(config);

        let request = Request::builder()
            .uri("/does-not-exist")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
