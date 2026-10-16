//! Versioned loopback API; deployment requires the documented HTTPS proxy.
use crate::{
    control::{ControlError, Mutation},
    store::Store,
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;

#[derive(Clone)]
struct App {
    store: Arc<Mutex<Store>>,
    capacity: Arc<Semaphore>,
    requests: Arc<Semaphore>,
}
#[derive(Serialize)]
struct Failure {
    error: &'static str,
}
fn fail(status: StatusCode, error: &'static str) -> Response {
    let mut response = (status, Json(Failure { error })).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        header::HeaderValue::from_static("nosniff"),
    );
    response
}
impl IntoResponse for ControlError {
    fn into_response(self) -> Response {
        match self {
            Self::Storage => fail(StatusCode::INTERNAL_SERVER_ERROR, "storage_failed"),
            Self::Invalid => fail(StatusCode::BAD_REQUEST, "invalid_input"),
            Self::Conflict => fail(StatusCode::CONFLICT, "revision_conflict"),
            Self::Forbidden => fail(StatusCode::FORBIDDEN, "forbidden"),
            Self::NotReady => fail(StatusCode::SERVICE_UNAVAILABLE, "not_ready"),
        }
    }
}
pub fn router(store: Store) -> Router {
    let app = App {
        store: Arc::new(Mutex::new(store)),
        capacity: Arc::new(Semaphore::new(16)),
        requests: Arc::new(Semaphore::new(16)),
    };
    Router::new()
        .route(
            "/healthz",
            get(|| async { Json(serde_json::json!({"status":"ok"})) }),
        )
        .route("/readyz", get(readiness))
        .route("/v1/directory", get(directory))
        .route("/v1/mutations", post(mutate))
        .fallback(|| async { fail(StatusCode::NOT_FOUND, "not_found") })
        .method_not_allowed_fallback(|| async {
            fail(StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed")
        })
        .layer(DefaultBodyLimit::max(128 * 1024))
        .layer(middleware::from_fn_with_state(app.clone(), bounds))
        .with_state(app)
}
async fn bounds(State(app): State<App>, request: Request, next: Next) -> Response {
    let Ok(_permit) = app.requests.try_acquire() else {
        return fail(StatusCode::SERVICE_UNAVAILABLE, "busy");
    };
    if request.headers().contains_key(header::ORIGIN) {
        return fail(StatusCode::FORBIDDEN, "browser_origin_forbidden");
    }
    let mut response =
        match tokio::time::timeout(std::time::Duration::from_secs(10), next.run(request)).await {
            Ok(response) => response,
            Err(_) => fail(StatusCode::REQUEST_TIMEOUT, "request_timeout"),
        };
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        header::HeaderValue::from_static("nosniff"),
    );
    response
}
async fn work<F>(app: App, operation: F) -> Response
where
    F: FnOnce(&mut Store) -> Response + Send + 'static,
{
    let Ok(permit) = app.capacity.clone().try_acquire_owned() else {
        return fail(StatusCode::SERVICE_UNAVAILABLE, "busy");
    };
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        match app.store.lock() {
            Ok(mut store) => operation(&mut store),
            Err(_) => ControlError::Storage.into_response(),
        }
    })
    .await
    .unwrap_or_else(|_| ControlError::Storage.into_response())
}
async fn readiness(State(app): State<App>) -> Response {
    work(app, |store| match (store.initialized(), store.ready()) {
        (Ok(true), Ok(true)) => Json(serde_json::json!({"status":"ready"})).into_response(),
        _ => ControlError::NotReady.into_response(),
    })
    .await
}
fn bearer(request: &Request) -> Option<zeroize::Zeroizing<String>> {
    if request
        .headers()
        .get_all(header::AUTHORIZATION)
        .iter()
        .count()
        != 1
    {
        return None;
    }
    let value = request
        .headers()
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")?;
    crate::control::valid_token(value).then(|| zeroize::Zeroizing::new(value.to_owned()))
}
async fn directory(State(app): State<App>, request: Request) -> Response {
    let Some(secret) = bearer(&request) else {
        return fail(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    work(app, move |store| match store.authenticate(&secret) {
        Ok(Some(principal)) => match store.directory(principal) {
            Ok(data) => Json(data).into_response(),
            Err(e) => e.into_response(),
        },
        Ok(None) => fail(StatusCode::UNAUTHORIZED, "unauthorized"),
        Err(_) => ControlError::Storage.into_response(),
    })
    .await
}
async fn mutate(State(app): State<App>, request: Request) -> Response {
    let Some(secret) = bearer(&request) else {
        return fail(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    // Authentication is rechecked with the mutation in one serialized worker.
    // Sanitize JSON rejection bodies; framework diagnostics can include input.
    let (parts, body) = request.into_parts();
    let request = Request::from_parts(parts, body);
    use axum::extract::FromRequest;
    let mutation = match Json::<Mutation>::from_request(request, &()).await {
        Ok(Json(m)) => m,
        Err(e) if e.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            return fail(StatusCode::PAYLOAD_TOO_LARGE, "body_too_large");
        }
        Err(_) => return fail(StatusCode::BAD_REQUEST, "invalid_json"),
    };
    work(app, move |store| match store.authenticate(&secret) {
        Ok(Some(principal)) => match store.mutate(principal, mutation) {
            Ok(data) => Json(data).into_response(),
            Err(e) => e.into_response(),
        },
        Ok(None) => fail(StatusCode::UNAUTHORIZED, "unauthorized"),
        Err(_) => ControlError::Storage.into_response(),
    })
    .await
}
