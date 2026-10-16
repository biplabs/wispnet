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
    enrollment_rate: Arc<Mutex<(std::time::Instant, u32)>>,
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
            Self::RateLimited => fail(StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            Self::NotReady => fail(StatusCode::SERVICE_UNAVAILABLE, "not_ready"),
        }
    }
}
pub fn router(store: Store) -> Router {
    let app = App {
        store: Arc::new(Mutex::new(store)),
        capacity: Arc::new(Semaphore::new(16)),
        requests: Arc::new(Semaphore::new(16)),
        enrollment_rate: Arc::new(Mutex::new((std::time::Instant::now(), 0))),
    };
    Router::new()
        .route(
            "/healthz",
            get(|| async { Json(serde_json::json!({"status":"ok"})) }),
        )
        .route("/readyz", get(readiness))
        .route("/v1/directory", get(directory))
        .route("/v1/mutations", post(mutate))
        .route("/v1/enrollment", get(enrollments))
        .route("/v1/enrollment/admin", post(enrollment_admin))
        .route("/v1/enrollment/challenge", post(enrollment_challenge))
        .route("/v1/enrollment/proof", post(enrollment_proof))
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
    if matches!(
        request.uri().path(),
        "/v1/enrollment/challenge" | "/v1/enrollment/proof"
    ) {
        let Ok(mut budget) = app.enrollment_rate.lock() else {
            return ControlError::Storage.into_response();
        };
        if budget.0.elapsed() >= std::time::Duration::from_secs(60) {
            *budget = (std::time::Instant::now(), 0);
        }
        if budget.1 >= 60 {
            return ControlError::RateLimited.into_response();
        }
        budget.1 += 1;
    }
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

async fn json<T: serde::de::DeserializeOwned>(request: Request) -> Result<T, Response> {
    use axum::extract::FromRequest;
    match Json::<T>::from_request(request, &()).await {
        Ok(Json(value)) => Ok(value),
        Err(e) if e.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            Err(fail(StatusCode::PAYLOAD_TOO_LARGE, "body_too_large"))
        }
        Err(_) => Err(fail(StatusCode::BAD_REQUEST, "invalid_json")),
    }
}
async fn enrollments(State(app): State<App>, request: Request) -> Response {
    let Some(secret) = bearer(&request) else {
        return fail(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    work(app, move |store| match store.authenticate(&secret) {
        Ok(Some(crate::control::Principal::Owner)) => {
            match store.enrollments(crate::enrollment::now()) {
                Ok(data) => Json(data).into_response(),
                Err(e) => e.into_response(),
            }
        }
        Ok(Some(_)) => ControlError::Forbidden.into_response(),
        _ => fail(StatusCode::UNAUTHORIZED, "unauthorized"),
    })
    .await
}
async fn enrollment_admin(State(app): State<App>, request: Request) -> Response {
    let Some(secret) = bearer(&request) else {
        return fail(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    let operation = match json::<crate::enrollment::Admin>(request).await {
        Ok(value) => value,
        Err(e) => return e,
    };
    work(app, move |store| match store.authenticate(&secret) {
        Ok(Some(crate::control::Principal::Owner)) => {
            match store.enrollment_admin(operation, crate::enrollment::now()) {
                Ok(data) => Json(data).into_response(),
                Err(e) => e.into_response(),
            }
        }
        Ok(Some(_)) => ControlError::Forbidden.into_response(),
        _ => fail(StatusCode::UNAUTHORIZED, "unauthorized"),
    })
    .await
}
async fn enrollment_challenge(State(app): State<App>, request: Request) -> Response {
    let Some(secret) = bearer(&request) else {
        return fail(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    let claim = match json::<wipsnet_core::enrollment::Claim>(request).await {
        Ok(value) => value,
        Err(e) => return e,
    };
    work(app, move |store| {
        match store.enrollment_challenge(&secret, claim, crate::enrollment::now()) {
            Ok(data) => Json(data).into_response(),
            Err(e) => e.into_response(),
        }
    })
    .await
}
async fn enrollment_proof(State(app): State<App>, request: Request) -> Response {
    let Some(secret) = bearer(&request) else {
        return fail(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    let proof = match json::<wipsnet_core::enrollment::Proof>(request).await {
        Ok(value) => value,
        Err(e) => return e,
    };
    work(app, move |store| {
        match store.enrollment_proof(&secret, proof, crate::enrollment::now()) {
            Ok(data) => Json(data).into_response(),
            Err(e) => e.into_response(),
        }
    })
    .await
}
