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
use std::time::Instant;
use tower_http::trace::TraceLayer;

struct AppState {
    config: Config,
    http_client: reqwest::Client,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
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
    let method = req.method().clone();

    // Attempt to extract a trace ID from the incoming headers, or default to "unknown"
    let trace_id = req
        .headers()
        .get("x-request-id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown");

    let endpoint = state
        .config
        .endpoints
        .iter()
        .find(|e| path.starts_with(&e.path));

    let Some(endpoint) = endpoint else {
        // STRUCTURED LOG: 404 Error
        tracing::warn!(
            "trace_id" = trace_id,
            "http.path" = %path,
            "http.method" = %method,
            "http.status_code" = 404,
            "No routing config found for path"
        );
        return Err(StatusCode::NOT_FOUND);
    };

    // =======================================
    // Dynamic Auth
    // =======================================
    if let Some(auth) = &endpoint.auth {
        let header_value = req.headers().get(&auth.header_name);
        let is_authorized = match header_value {
            Some(val) => val.to_str().unwrap_or("") == auth.expected_value,
            None => false,
        };

        if !is_authorized {
            // STRUCTURED LOG: Auth Failure
            tracing::warn!(
                "trace_id" = trace_id,
                "http.path" = %path,
                "gateway.auth_status" = "failed",
                "gateway.auth_header" = %auth.header_name,
                "http.status_code" = 401,
                "Unauthorized access attempt"
            );
            return Err(StatusCode::UNAUTHORIZED);
        }
    }

    let downstream_url = if query.is_empty() {
        format!("{}{}", endpoint.base, path)
    } else {
        format!("{}{}?{}", endpoint.base, path, query)
    };

    let mut headers = req.headers().clone();
    headers.remove(axum::http::header::HOST);

    let body_bytes = axum::body::to_bytes(req.into_body(), usize::MAX)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    // Start timing the request
    let start = Instant::now();

    let res = state
        .http_client
        .request(method.clone(), &downstream_url)
        .headers(headers)
        .body(body_bytes)
        .send()
        .await
        .map_err(|e| {
            // STRUCTURED LOG: Upstream Failure
            tracing::error!(
                "trace_id" = trace_id,
                "http.path" = %path,
                "gateway.upstream_url" = %downstream_url,
                "error" = %e,
                "http.status_code" = 502,
                "Upstream request failed"
            );
            StatusCode::BAD_GATEWAY
        })?;

    let request_time = start.elapsed();
    let status_code = res.status();

    let mut response_builder = axum::http::Response::builder().status(status_code);
    if let Some(headers_mut) = response_builder.headers_mut() {
        for (k, v) in res.headers() {
            headers_mut.insert(k, v.clone());
        }
    }

    let res_bytes = res
        .bytes()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // STRUCTURED LOG: Successful Request
    // This perfectly matches the "Universal Metadata" and "Request & Response Context" guide
    tracing::info!(
        "trace_id" = trace_id,
        "http.method" = %method,
        "http.path" = %path,
        "http.status_code" = status_code.as_u16(),
        "http.latency_ms" = request_time.as_millis(),
        "gateway.upstream_url" = %downstream_url,
        "HTTP request processed"
    );

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
