use axum::{
    body::Body,
    extract::{Path, State},
    http::{
        header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE, PRAGMA},
        HeaderMap, HeaderValue, StatusCode,
    },
    response::Response,
    Json,
};
use serde::Deserialize;
use std::path::{Path as FsPath, PathBuf};

use crate::{
    core::OperationOrigin,
    domain::{LegacyMigrationDestination, LegacyMigrationPreflight, LegacyMigrationStatus},
    service::{
        legacy_migration::{LegacyMigrationRequest, SecretLegacyKey},
        ServiceError,
    },
};

use super::{parse_json, require_session, ApiError, AppState, JsonRejection};

#[derive(Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
enum DestinationRequest {
    CurrentNode,
    ExistingConfig { path: PathBuf },
    CreateNew,
}

impl From<DestinationRequest> for LegacyMigrationDestination {
    fn from(value: DestinationRequest) -> Self {
        match value {
            DestinationRequest::CurrentNode => Self::CurrentNode,
            DestinationRequest::ExistingConfig { path } => Self::ExistingConfig(path),
            DestinationRequest::CreateNew => Self::CreateNew,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PreflightRequest {
    old_peer_private_key: String,
    destination: DestinationRequest,
    #[serde(default)]
    allow_initialize_recipient: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LookupRequest {
    old_peer_private_key: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StartRequest {
    approved: LegacyMigrationPreflight,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AdvanceRequest {
    old_peer_private_key: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RebroadcastRequest {
    confirmed: bool,
}

pub(super) async fn preflight(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: Result<Json<PreflightRequest>, JsonRejection>,
) -> Result<Json<LegacyMigrationPreflight>, ApiError> {
    require_session(&headers, &state.auth)?;
    let request = parse_json(payload)?;
    let source = SecretLegacyKey::parse(&request.old_peer_private_key).map_err(|error| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("invalid old Ed448 peer key: {error}"),
        )
    })?;
    let request = LegacyMigrationRequest::new(
        source,
        request.destination.into(),
        request.allow_initialize_recipient,
    );
    state
        .core
        .preflight_legacy_migration(request)
        .await
        .map(Json)
        .map_err(service_error)
}

pub(super) async fn lookup(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: Result<Json<LookupRequest>, JsonRejection>,
) -> Result<Json<LegacyMigrationStatus>, ApiError> {
    require_session(&headers, &state.auth)?;
    let request = parse_json(payload)?;
    let source = SecretLegacyKey::parse(&request.old_peer_private_key).map_err(|error| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("invalid old Ed448 peer key: {error}"),
        )
    })?;
    let fingerprint = source.fingerprint();
    let status = state
        .core
        .load_legacy_migration(fingerprint)
        .await
        .map_err(service_error)?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "legacy migration was not found"))?;
    if status.source.fingerprint != fingerprint {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "legacy migration identity does not match its journal",
        ));
    }
    Ok(Json(status))
}

pub(super) async fn start(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: Result<Json<StartRequest>, JsonRejection>,
) -> Result<Json<LegacyMigrationStatus>, ApiError> {
    require_session(&headers, &state.auth)?;
    let request = parse_json(payload)?;
    state
        .core
        .start_legacy_migration(OperationOrigin::Web, request.approved)
        .await
        .map(Json)
        .map_err(service_error)
}

pub(super) async fn status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(fingerprint): Path<String>,
) -> Result<Json<LegacyMigrationStatus>, ApiError> {
    require_session(&headers, &state.auth)?;
    validate_fingerprint(&fingerprint)?;
    state
        .core
        .load_legacy_migration(&fingerprint)
        .await
        .map_err(service_error)?
        .map(Json)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "legacy migration was not found"))
}

pub(super) async fn advance(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(fingerprint): Path<String>,
    payload: Result<Json<AdvanceRequest>, JsonRejection>,
) -> Result<Json<LegacyMigrationStatus>, ApiError> {
    require_session(&headers, &state.auth)?;
    validate_fingerprint(&fingerprint)?;
    let request = parse_json(payload)?;
    let source = request
        .old_peer_private_key
        .map(|value| {
            SecretLegacyKey::parse(&value).map_err(|error| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    format!("invalid old Ed448 peer key: {error}"),
                )
            })
        })
        .transpose()?;
    state
        .core
        .advance_legacy_migration(OperationOrigin::Web, &fingerprint, source)
        .await
        .map(Json)
        .map_err(service_error)
}

pub(super) async fn rebroadcast(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(fingerprint): Path<String>,
    payload: Result<Json<RebroadcastRequest>, JsonRejection>,
) -> Result<Json<LegacyMigrationStatus>, ApiError> {
    require_session(&headers, &state.auth)?;
    validate_fingerprint(&fingerprint)?;
    let request = parse_json(payload)?;
    if !request.confirmed {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "explicit confirmation is required before retrying a saved migration",
        ));
    }
    state
        .core
        .rebroadcast_legacy_migration(OperationOrigin::Web, &fingerprint, true)
        .await
        .map(Json)
        .map_err(service_error)
}

pub(super) async fn download(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((fingerprint, kind)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    require_session(&headers, &state.auth)?;
    validate_fingerprint(&fingerprint)?;
    let filename = match kind.as_str() {
        "config" => "config.yml",
        "keys" => "keys.yml",
        _ => {
            return Err(ApiError::new(
                StatusCode::NOT_FOUND,
                "unknown migration file",
            ))
        }
    };
    let status = state
        .core
        .load_legacy_migration(&fingerprint)
        .await
        .map_err(service_error)?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "legacy migration was not found"))?;
    if status.source.fingerprint != fingerprint {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "legacy migration identity does not match its journal",
        ));
    }
    let home = state.home.clone();
    let bytes = tokio::task::spawn_blocking(move || generated_file(&home, &status, filename))
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "file read was interrupted",
            )
        })??;
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "application/yaml; charset=utf-8")
        .header(CACHE_CONTROL, "no-store")
        .header(PRAGMA, "no-cache")
        .header("x-content-type-options", "nosniff")
        .header(
            CONTENT_DISPOSITION,
            HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
                .expect("fixed migration filename is a valid header"),
        )
        .body(Body::from(bytes))
        .map_err(|_| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "cannot build file response",
            )
        })
}

fn generated_file(
    home: &FsPath,
    status: &LegacyMigrationStatus,
    filename: &str,
) -> Result<Vec<u8>, ApiError> {
    if !status.source_key_persisted || status.recipient_config_dir != status.task_dir {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "only generated migration packages can be downloaded",
        ));
    }
    let task = std::fs::canonicalize(&status.task_dir)
        .map_err(|_| ApiError::new(StatusCode::NOT_FOUND, "migration package is unavailable"))?;
    let home = std::fs::canonicalize(home).map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Console home is unavailable",
        )
    })?;
    let expected_directory = format!("ed448-{}", status.source.fingerprint);
    if !task.starts_with(home.join(".qclient-console/legacy-wallets"))
        || task.file_name().and_then(|value| value.to_str()) != Some(expected_directory.as_str())
    {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "migration package path is not downloadable",
        ));
    }
    let path = match filename {
        "config.yml" => &status.config_path,
        "keys.yml" => &status.keys_path,
        _ => unreachable!("download filenames are fixed by the route"),
    };
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| ApiError::new(StatusCode::NOT_FOUND, "migration file is unavailable"))?;
    if !metadata.file_type().is_file() {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "migration file is not a regular file",
        ));
    }
    let path = std::fs::canonicalize(path)
        .map_err(|_| ApiError::new(StatusCode::NOT_FOUND, "migration file is unavailable"))?;
    if path != task.join(filename) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "migration file path is not bound to its generated package",
        ));
    }
    std::fs::read(path).map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "cannot read migration file",
        )
    })
}

fn validate_fingerprint(fingerprint: &str) -> Result<(), ApiError> {
    if fingerprint.len() != 32
        || !fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid legacy migration fingerprint",
        ));
    }
    Ok(())
}

fn service_error(error: ServiceError) -> ApiError {
    let status = match &error {
        ServiceError::InvalidInput(_) => StatusCode::BAD_REQUEST,
        ServiceError::Conflict(_) => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    ApiError::new(status, error.to_string())
}
