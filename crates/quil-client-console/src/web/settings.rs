use axum::{
    extract::{rejection::JsonRejection, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::{
    i18n::Locale,
    service::read_rpc,
    settings::{self, ConsoleSettings, WebSettings},
};

use super::{parse_json, require_session, ApiError, AppState};

#[derive(Serialize)]
pub(super) struct SettingsResponse {
    enabled: bool,
    port: u16,
    language: Locale,
    supported_languages: [LanguageOption; 4],
    restart_required: bool,
    read_rpc: Option<String>,
    effective_read_rpc: Option<String>,
    read_rpc_file: Option<String>,
}

#[derive(Serialize)]
struct LanguageOption {
    id: Locale,
    label: &'static str,
}

#[derive(Deserialize)]
pub(super) struct SettingsRequest {
    enabled: bool,
    port: u16,
    language: Locale,
    #[serde(default, deserialize_with = "deserialize_optional_read_rpc")]
    read_rpc: Option<Option<String>>,
}

fn deserialize_optional_read_rpc<'de, D>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

struct ReadRpcSettings {
    override_address: Option<String>,
    effective_address: Option<String>,
    file: Option<String>,
}

pub(super) async fn read(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<SettingsResponse>, ApiError> {
    require_session(&headers, &state.auth)?;
    let home = state.home.clone();
    let config_dir = state.core.state().active_config.map(|config| config.path);
    let (settings, read_rpc) = tokio::task::spawn_blocking(move || {
        Ok::<_, String>((
            settings::load(&home)?,
            read_rpc_settings(config_dir.as_deref())?,
        ))
    })
    .await
    .map_err(|error| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("settings load task failed: {error}"),
        )
    })?
    .map_err(|error| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error))?;
    Ok(Json(response(settings, false, read_rpc)))
}

pub(super) async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: Result<Json<SettingsRequest>, JsonRejection>,
) -> Result<Json<SettingsResponse>, ApiError> {
    require_session(&headers, &state.auth)?;
    let request = parse_json(payload)?;
    settings::parse_port(&request.port.to_string())
        .map_err(|error| ApiError::new(StatusCode::BAD_REQUEST, error))?;
    if let Some(value) = request
        .read_rpc
        .as_ref()
        .and_then(Option::as_deref)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        read_rpc::validate(value)
            .map_err(|error| ApiError::new(StatusCode::BAD_REQUEST, error.to_string()))?;
    }
    let candidate = ConsoleSettings {
        web: WebSettings {
            enabled: request.enabled,
            port: request.port,
        },
        language: request.language,
    };
    let home = state.home.clone();
    let config_dir = state
        .core
        .state()
        .active_config
        .map(|config| config.path)
        .ok_or_else(|| {
            ApiError::new(StatusCode::BAD_REQUEST, "Select a node configuration first")
        })?;
    let requested_read_rpc = request.read_rpc;
    let (previous, read_rpc) = tokio::task::spawn_blocking(move || {
        let previous = settings::load(&home)?;
        let previous_read_rpc =
            read_rpc::load_override(&config_dir).map_err(|error| error.to_string())?;
        settings::save(&home, candidate)?;
        if let Some(requested_read_rpc) = requested_read_rpc.as_ref() {
            if let Err(error) = read_rpc::save(&config_dir, requested_read_rpc.as_deref()) {
                let _ = settings::save(&home, previous);
                return Err(error.to_string());
            }
        }
        let current = match read_rpc_settings(Some(&config_dir)) {
            Ok(current) => current,
            Err(error) => {
                let _ = settings::save(&home, previous);
                let _ = read_rpc::save(&config_dir, previous_read_rpc.as_deref());
                return Err(error);
            }
        };
        Ok::<_, String>((previous, current))
    })
    .await
    .map_err(|error| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("settings save task failed: {error}"),
        )
    })?
    .map_err(|error| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error))?;
    Ok(Json(response(
        candidate,
        previous.web != candidate.web,
        read_rpc,
    )))
}

fn response(
    settings: ConsoleSettings,
    restart_required: bool,
    read_rpc: ReadRpcSettings,
) -> SettingsResponse {
    SettingsResponse {
        enabled: settings.web.enabled,
        port: settings.web.port,
        language: settings.language,
        supported_languages: [
            language(Locale::English),
            language(Locale::TraditionalChinese),
            language(Locale::Japanese),
            language(Locale::French),
        ],
        restart_required,
        read_rpc: read_rpc.override_address,
        effective_read_rpc: read_rpc.effective_address,
        read_rpc_file: read_rpc.file,
    }
}

fn read_rpc_settings(config_dir: Option<&std::path::Path>) -> Result<ReadRpcSettings, String> {
    let Some(config_dir) = config_dir else {
        return Ok(ReadRpcSettings {
            override_address: None,
            effective_address: None,
            file: None,
        });
    };
    let config = quil_config::load_config(config_dir).map_err(|error| error.to_string())?;
    let override_address =
        read_rpc::load_override(config_dir).map_err(|error| error.to_string())?;
    let effective_address = override_address
        .clone()
        .unwrap_or_else(|| config.effective_listen_grpc_multiaddr().into());
    Ok(ReadRpcSettings {
        override_address,
        effective_address: Some(effective_address),
        file: Some(read_rpc::file(config_dir).to_string_lossy().into_owned()),
    })
}

fn language(id: Locale) -> LanguageOption {
    LanguageOption {
        id,
        label: id.display_name(),
    }
}
