use std::collections::BTreeMap;

use axum::{
    extract::{rejection::PathRejection, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::{
    capability::{CapabilitySpec, CapabilityTarget, CapabilityTask},
    core::OperationOrigin,
    service::ServiceError,
};

use super::{parse_json, require_session, ApiError, AppState, JsonRejection};

#[derive(Deserialize)]
pub(super) struct StartCapabilityRequest {
    values: BTreeMap<String, Vec<String>>,
    confirmed: bool,
}

#[derive(Serialize)]
struct NativeCapabilityResponse {
    route: String,
}

pub(super) async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<CapabilitySpec>>, ApiError> {
    require_session(&headers, &state.auth)?;
    Ok(Json(state.core.capabilities().iter().cloned().collect()))
}

pub(super) async fn start(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    headers: HeaderMap,
    payload: Result<Json<StartCapabilityRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    require_session(&headers, &state.auth)?;
    let Path(id) = path.map_err(path_error)?;
    let target = state
        .core
        .capabilities()
        .get(&id)
        .map(|capability| capability.target.clone())
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, format!("unknown capability: {id}")))?;
    let request = parse_json(payload)?;

    match target {
        CapabilityTarget::Native { route } => {
            Ok((StatusCode::OK, Json(NativeCapabilityResponse { route })).into_response())
        }
        CapabilityTarget::EmbeddedCli { .. } => {
            let task = state
                .core
                .enqueue_capability(OperationOrigin::Web, &id, request.values, request.confirmed)
                .map_err(capability_error)?;
            Ok((StatusCode::ACCEPTED, Json(task)).into_response())
        }
    }
}

pub(super) async fn task(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    headers: HeaderMap,
) -> Result<Json<CapabilityTask>, ApiError> {
    require_session(&headers, &state.auth)?;
    let Path(id) = path.map_err(path_error)?;
    let id = id.parse::<u64>().map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "capability task id must be an unsigned integer",
        )
    })?;
    state.core.capability_task(id).map(Json).ok_or_else(|| {
        ApiError::new(
            StatusCode::NOT_FOUND,
            format!("unknown capability task: {id}"),
        )
    })
}

fn path_error(_error: PathRejection) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "invalid request path")
}

fn capability_error(error: ServiceError) -> ApiError {
    let status = match &error {
        ServiceError::InvalidInput(_) => StatusCode::BAD_REQUEST,
        ServiceError::Conflict(_) => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    let message = if status == StatusCode::INTERNAL_SERVER_ERROR {
        "internal capability error".to_owned()
    } else {
        error.to_string()
    };
    ApiError::new(status, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_enqueue_internal_errors_do_not_expose_sensitive_details() {
        let secret = "catalog initialization failed at /secret/qclient/path";

        let error = capability_error(ServiceError::Operation(secret.into()));

        assert_eq!(error.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(error.message, "internal capability error");
        assert!(!error.message.contains(secret));
    }
}
