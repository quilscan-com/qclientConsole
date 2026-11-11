mod auth;
mod capabilities;
mod legacy_migration;
mod mint;
mod pages;
mod settings;

use std::{io, net::SocketAddr, path::PathBuf, sync::Arc};

use auth::{AuthError, AuthStore};
use axum::{
    body::Body,
    extract::{rejection::JsonRejection, State},
    http::{
        header::{CONTENT_TYPE, COOKIE, SET_COOKIE},
        HeaderMap, HeaderValue, StatusCode,
    },
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};

use crate::core::ConsoleCore;

const SESSION_COOKIE: &str = "qclient_session";
const SHUTDOWN_GRACE_PERIOD: std::time::Duration = std::time::Duration::from_millis(250);
const INDEX: &str = include_str!("../../web/dist/index.html");
const APP_CSS: &str = include_str!("../../web/dist/assets/app.css");
const APP_JS: &str = include_str!("../../web/dist/assets/app.js");

pub struct WebServer;

pub struct WebServerHandle {
    local_addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<io::Result<()>>>,
}

impl WebServer {
    pub async fn start(
        address: SocketAddr,
        core: ConsoleCore,
        home: PathBuf,
    ) -> io::Result<WebServerHandle> {
        let listener = TcpListener::bind(address).await?;
        let local_addr = listener.local_addr()?;
        let state = AppState {
            core,
            auth: Arc::new(AuthStore::new(home.clone())?),
            home,
            mint_tasks: mint::ActiveMintTasks::default(),
        };
        mint::resume(state.clone());
        let app = Router::new()
            .route("/", get(index))
            .route("/assets/app.css", get(app_css))
            .route("/assets/app.js", get(app_js))
            .route("/api/v1/bootstrap", get(bootstrap))
            .route("/api/v1/setup", post(setup))
            .route("/api/v1/login", post(login))
            .route("/api/v1/logout", post(logout))
            .route("/api/v1/session", get(session))
            .route("/api/v1/context", get(context))
            .route("/api/v1/capabilities", get(capabilities::list))
            .route("/api/v1/capabilities/:id/start", post(capabilities::start))
            .route("/api/v1/capability-tasks/:id", get(capabilities::task))
            .route(
                "/api/v1/settings",
                get(settings::read).put(settings::update),
            )
            .route("/api/v1/pages/:slug", get(pages::page))
            .route("/api/v1/rewards/mint", post(mint::start))
            .route("/api/v1/rewards/mint/rebroadcast", post(mint::rebroadcast))
            .route(
                "/api/v1/legacy-migrations/preflight",
                post(legacy_migration::preflight),
            )
            .route(
                "/api/v1/legacy-migrations/lookup",
                post(legacy_migration::lookup),
            )
            .route(
                "/api/v1/legacy-migrations/start",
                post(legacy_migration::start),
            )
            .route(
                "/api/v1/legacy-migrations/:fingerprint",
                get(legacy_migration::status),
            )
            .route(
                "/api/v1/legacy-migrations/:fingerprint/advance",
                post(legacy_migration::advance),
            )
            .route(
                "/api/v1/legacy-migrations/:fingerprint/rebroadcast",
                post(legacy_migration::rebroadcast),
            )
            .route(
                "/api/v1/legacy-migrations/:fingerprint/files/:kind",
                get(legacy_migration::download),
            )
            .with_state(state);
        let (shutdown, stopped) = oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
        });

        Ok(WebServerHandle {
            local_addr,
            shutdown: Some(shutdown),
            task: Some(task),
        })
    }
}

impl WebServerHandle {
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub async fn shutdown(mut self) -> io::Result<()> {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        let mut task = self.task.take().expect("server task is always present");
        match tokio::time::timeout(SHUTDOWN_GRACE_PERIOD, &mut task).await {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => Err(io::Error::other(error)),
            Err(_) => {
                task.abort();
                match task.await {
                    Ok(result) => result,
                    Err(error) if error.is_cancelled() => Ok(()),
                    Err(error) => Err(io::Error::other(error)),
                }
            }
        }
    }
}

impl Drop for WebServerHandle {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

#[derive(Clone)]
struct AppState {
    core: ConsoleCore,
    auth: Arc<AuthStore>,
    home: PathBuf,
    mint_tasks: mint::ActiveMintTasks,
}

#[derive(Serialize)]
struct BootstrapResponse {
    username: &'static str,
    initialized: bool,
}

#[derive(Deserialize)]
struct SetupRequest {
    password: String,
    password_confirmation: String,
}

#[derive(Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Serialize)]
struct SessionResponse {
    username: &'static str,
    authenticated: bool,
}

#[derive(Serialize)]
struct ContextResponse {
    active_config_name: Option<String>,
    active_config_path: Option<String>,
    connected: Option<bool>,
    rpc_address: Option<String>,
    peer_id: Option<String>,
    last_received_frame: Option<u64>,
    last_global_head_frame: Option<u64>,
    running_workers: Option<u32>,
    allocated_workers: Option<u32>,
    allocation_count: Option<usize>,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "authentication required")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorResponse {
                error: self.message,
            }),
        )
            .into_response()
    }
}

async fn index() -> Html<&'static str> {
    Html(INDEX)
}

async fn app_css() -> Response {
    asset("text/css; charset=utf-8", APP_CSS)
}

async fn app_js() -> Response {
    asset("text/javascript; charset=utf-8", APP_JS)
}

fn asset(content_type: &'static str, contents: &'static str) -> Response {
    Response::builder()
        .header(CONTENT_TYPE, content_type)
        .body(Body::from(contents))
        .expect("static asset response is valid")
}

async fn bootstrap(State(state): State<AppState>) -> Json<BootstrapResponse> {
    Json(BootstrapResponse {
        username: "admin",
        initialized: state.auth.initialized(),
    })
}

async fn setup(
    State(state): State<AppState>,
    payload: Result<Json<SetupRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    if state.auth.initialized() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "password is already initialized",
        ));
    }
    let request = parse_json(payload)?;
    if request.password.chars().count() < 10 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "password must be at least 10 characters",
        ));
    }
    if request.password != request.password_confirmation {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "password confirmation does not match",
        ));
    }
    let auth = state.auth.clone();
    let password = request.password;
    let setup = tokio::task::spawn_blocking(move || auth.setup(&password))
        .await
        .map_err(|error| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("password setup task failed: {error}"),
            )
        })?;
    match setup {
        Ok(()) => {}
        Err(AuthError::AlreadyInitialized) => {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "password is already initialized",
            ));
        }
        Err(AuthError::Storage(error)) => {
            return Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                error.to_string(),
            ));
        }
    }

    Ok(session_response(&state.auth.create_session()))
}

async fn login(
    State(state): State<AppState>,
    payload: Result<Json<LoginRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    let request = parse_json(payload)?;
    let auth = state.auth.clone();
    let authenticated = tokio::task::spawn_blocking(move || {
        request.username == "admin" && auth.authenticate(&request.password)
    })
    .await
    .map_err(|error| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("password verification task failed: {error}"),
        )
    })?;
    if !authenticated {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "invalid username or password",
        ));
    }

    Ok(session_response(&state.auth.create_session()))
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    let token = require_session(&headers, &state.auth)?;
    state.auth.logout(&token);
    let mut response = Json(SessionResponse {
        username: "admin",
        authenticated: false,
    })
    .into_response();
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_static("qclient_session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0"),
    );
    Ok(response)
}

async fn session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<SessionResponse>, ApiError> {
    require_session(&headers, &state.auth)?;
    Ok(Json(SessionResponse {
        username: "admin",
        authenticated: true,
    }))
}

async fn context(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ContextResponse>, ApiError> {
    require_session(&headers, &state.auth)?;
    let core = state.core.state();
    let active_config_name = core.active_config.as_ref().map(|entry| entry.name.clone());
    let active_config_path = core
        .active_config
        .as_ref()
        .map(|entry| entry.path.to_string_lossy().into_owned());
    let snapshot = core.node_snapshot.as_ref();

    Ok(Json(ContextResponse {
        active_config_name,
        active_config_path,
        connected: snapshot.map(|snapshot| snapshot.connected),
        rpc_address: snapshot.map(|snapshot| snapshot.rpc_address.clone()),
        peer_id: snapshot.map(|snapshot| snapshot.peer_id.clone()),
        last_received_frame: snapshot.map(|snapshot| snapshot.last_received_frame),
        last_global_head_frame: snapshot.map(|snapshot| snapshot.last_global_head_frame),
        running_workers: snapshot.map(|snapshot| snapshot.running_workers),
        allocated_workers: snapshot.map(|snapshot| snapshot.allocated_workers),
        allocation_count: snapshot.map(|snapshot| snapshot.allocations.len()),
    }))
}

fn parse_json<T>(payload: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    payload.map(|Json(value)| value).map_err(|error| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("invalid JSON request: {error}"),
        )
    })
}

fn require_session(headers: &HeaderMap, auth: &AuthStore) -> Result<String, ApiError> {
    let token = cookie(headers, SESSION_COOKIE).ok_or_else(ApiError::unauthorized)?;
    if !auth.validate(&token) {
        return Err(ApiError::unauthorized());
    }
    Ok(token)
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .map(str::trim)
        .find_map(|cookie| cookie.strip_prefix(&format!("{name}=")).map(str::to_owned))
}

fn session_response(token: &str) -> Response {
    let mut response = Json(SessionResponse {
        username: "admin",
        authenticated: true,
    })
    .into_response();
    let cookie = format!("{SESSION_COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/");
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str(&cookie).expect("session cookie contains only ASCII"),
    );
    response
}
