use {
    axum::{
        Extension, Json, Router,
        body::Body,
        extract::{DefaultBodyLimit, State},
        http::{HeaderMap, HeaderValue, StatusCode, header},
        response::{IntoResponse, Response},
        routing::{get, post},
    },
    base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD},
    futures::StreamExt,
    moltis_gateway::{
        auth::AuthIdentity,
        files::{FilesError, LocalFilesService},
    },
    moltis_protocol::scopes,
    serde::Deserialize,
    std::sync::Arc,
    tokio::io::AsyncWriteExt,
    tokio_util::io::ReaderStream,
};

use crate::AppState;

pub const MAX_FILE_UPLOAD_SIZE: u64 = 1024 * 1024 * 1024;
const FILE_PATH_HEADER: &str = "x-moltis-file-path";
const FILE_PATH_HEADER_PREFIX: &str = "base64url:";
const OVERWRITE_HEADER: &str = "x-moltis-overwrite";

#[derive(Clone, Copy)]
enum FilesAccess {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy)]
struct ApiError {
    status: StatusCode,
    message: &'static str,
}

impl ApiError {
    const fn new(status: StatusCode, message: &'static str) -> Self {
        Self { status, message }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        api_error(self.status, self.message)
    }
}

#[derive(Debug, Deserialize)]
struct DirectoryRequest {
    path: String,
}

#[derive(Debug, Deserialize)]
struct MoveRequest {
    source: String,
    destination: String,
    #[serde(default)]
    overwrite: bool,
}

#[derive(Debug, Deserialize)]
struct DeleteRequest {
    path: String,
    #[serde(default)]
    recursive: bool,
}

pub fn files_router() -> Router<AppState> {
    Router::new()
        .route("/entries", get(list_entries).delete(delete_entry))
        .route(
            "/content",
            get(download_file)
                .put(upload_file)
                .layer(DefaultBodyLimit::max(
                    usize::try_from(MAX_FILE_UPLOAD_SIZE).unwrap_or(usize::MAX),
                )),
        )
        .route("/directories", post(create_directory))
        .route("/move", post(move_entry))
}

async fn list_entries(
    State(state): State<AppState>,
    identity: Option<Extension<AuthIdentity>>,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = require_files_access(identity.as_deref(), FilesAccess::Read) {
        return response.into_response();
    }
    let path = match logical_path_header(&headers, false) {
        Ok(path) => path,
        Err(response) => return response.into_response(),
    };
    let service = match files_service(&state) {
        Ok(service) => service,
        Err(response) => return response.into_response(),
    };
    match service.list(&path) {
        Ok(listing) => Json(listing).into_response(),
        Err(error) => files_error_response(error),
    }
}

async fn upload_file(
    State(state): State<AppState>,
    identity: Option<Extension<AuthIdentity>>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    if let Err(response) = require_files_access(identity.as_deref(), FilesAccess::Write) {
        return response.into_response();
    }
    let path = match logical_path_header(&headers, true) {
        Ok(path) => path,
        Err(response) => return response.into_response(),
    };
    let overwrite = match overwrite_header(&headers) {
        Ok(overwrite) => overwrite,
        Err(response) => return response.into_response(),
    };
    if content_length_exceeds_limit(&headers, MAX_FILE_UPLOAD_SIZE) {
        return payload_too_large();
    }
    let service = match files_service(&state) {
        Ok(service) => service,
        Err(response) => return response.into_response(),
    };
    let _upload_slot = match service.acquire_upload_slot().await {
        Ok(permit) => permit,
        Err(error) => return files_error_response(error),
    };
    let mut pending = match service.begin_upload(&path, overwrite) {
        Ok(pending) => pending,
        Err(error) => return files_error_response(error),
    };

    let mut size = 0_u64;
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(_) => return api_error(StatusCode::BAD_REQUEST, "failed to read upload body"),
        };
        size = match checked_upload_size(size, chunk.len() as u64, MAX_FILE_UPLOAD_SIZE) {
            Some(size) => size,
            None => return payload_too_large(),
        };
        let writer = match pending.writer() {
            Ok(writer) => writer,
            Err(error) => return files_error_response(error),
        };
        if let Err(error) = writer.write_all(&chunk).await {
            tracing::warn!(%error, "failed to write managed file upload");
            return api_error(StatusCode::INTERNAL_SERVER_ERROR, "file upload failed");
        }
    }

    match pending.commit().await {
        Ok(entry) => (StatusCode::CREATED, Json(entry)).into_response(),
        Err(error) => files_error_response(error),
    }
}

async fn download_file(
    State(state): State<AppState>,
    identity: Option<Extension<AuthIdentity>>,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = require_files_access(identity.as_deref(), FilesAccess::Read) {
        return response.into_response();
    }
    let path = match logical_path_header(&headers, true) {
        Ok(path) => path,
        Err(response) => return response.into_response(),
    };
    let service = match files_service(&state) {
        Ok(service) => service,
        Err(response) => return response.into_response(),
    };
    let opened = match service.open_download(&path) {
        Ok(opened) => opened,
        Err(error) => return files_error_response(error),
    };
    let disposition = match content_disposition(&opened.name) {
        Ok(disposition) => disposition,
        Err(response) => return response.into_response(),
    };
    let Some(size) = opened.metadata.size_bytes else {
        return api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "file metadata unavailable",
        );
    };
    let content_length = match HeaderValue::from_str(&size.to_string()) {
        Ok(value) => value,
        Err(_) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "invalid file metadata"),
    };
    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    response_headers.insert(header::CONTENT_DISPOSITION, disposition);
    response_headers.insert(header::CONTENT_LENGTH, content_length);
    let stream = ReaderStream::new(tokio::fs::File::from_std(opened.file));
    (response_headers, Body::from_stream(stream)).into_response()
}

async fn create_directory(
    State(state): State<AppState>,
    identity: Option<Extension<AuthIdentity>>,
    Json(request): Json<DirectoryRequest>,
) -> Response {
    if let Err(response) = require_files_access(identity.as_deref(), FilesAccess::Write) {
        return response.into_response();
    }
    let service = match files_service(&state) {
        Ok(service) => service,
        Err(response) => return response.into_response(),
    };
    match service.create_directory(&request.path) {
        Ok(entry) => (StatusCode::CREATED, Json(entry)).into_response(),
        Err(error) => files_error_response(error),
    }
}

async fn move_entry(
    State(state): State<AppState>,
    identity: Option<Extension<AuthIdentity>>,
    Json(request): Json<MoveRequest>,
) -> Response {
    if let Err(response) = require_files_access(identity.as_deref(), FilesAccess::Write) {
        return response.into_response();
    }
    let service = match files_service(&state) {
        Ok(service) => service,
        Err(response) => return response.into_response(),
    };
    match service.move_entry(&request.source, &request.destination, request.overwrite) {
        Ok(entry) => Json(entry).into_response(),
        Err(error) => files_error_response(error),
    }
}

async fn delete_entry(
    State(state): State<AppState>,
    identity: Option<Extension<AuthIdentity>>,
    Json(request): Json<DeleteRequest>,
) -> Response {
    if let Err(response) = require_files_access(identity.as_deref(), FilesAccess::Write) {
        return response.into_response();
    }
    let service = match files_service(&state) {
        Ok(service) => service,
        Err(response) => return response.into_response(),
    };
    match service.delete(&request.path, request.recursive) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => files_error_response(error),
    }
}

fn files_service(state: &AppState) -> Result<Arc<LocalFilesService>, ApiError> {
    state.gateway.services.files.clone().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "managed files service unavailable",
        )
    })
}

fn logical_path_header(headers: &HeaderMap, required: bool) -> Result<String, ApiError> {
    match headers.get(FILE_PATH_HEADER) {
        Some(value) => {
            let encoded = value.to_str().map_err(|_| {
                ApiError::new(StatusCode::BAD_REQUEST, "invalid managed file path header")
            })?;
            let encoded = encoded
                .strip_prefix(FILE_PATH_HEADER_PREFIX)
                .ok_or_else(|| {
                    ApiError::new(
                        StatusCode::BAD_REQUEST,
                        "invalid managed file path encoding",
                    )
                })?;
            let bytes = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "invalid managed file path encoding",
                )
            })?;
            String::from_utf8(bytes).map_err(|_| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "invalid managed file path encoding",
                )
            })
        },
        None if required => Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "managed file path header required",
        )),
        None => Ok(String::new()),
    }
}

fn overwrite_header(headers: &HeaderMap) -> Result<bool, ApiError> {
    let Some(value) = headers.get(OVERWRITE_HEADER) else {
        return Ok(false);
    };
    let value = value
        .to_str()
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid overwrite header"))?;
    if value.eq_ignore_ascii_case("true") {
        Ok(true)
    } else if value.eq_ignore_ascii_case("false") {
        Ok(false)
    } else {
        Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "overwrite header must be true or false",
        ))
    }
}

fn require_files_access(
    identity: Option<&AuthIdentity>,
    access: FilesAccess,
) -> Result<(), ApiError> {
    let Some(identity) = identity else {
        return Ok(());
    };
    let required = match access {
        FilesAccess::Read => scopes::READ,
        FilesAccess::Write => scopes::WRITE,
    };
    if identity.has_scope(required) || identity.has_scope(scopes::ADMIN) {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "insufficient scope for managed files operation",
        ))
    }
}

fn content_length_exceeds_limit(headers: &HeaderMap, limit: u64) -> bool {
    headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length > limit)
}

fn checked_upload_size(current: u64, chunk: u64, limit: u64) -> Option<u64> {
    current
        .checked_add(chunk)
        .filter(|new_size| *new_size <= limit)
}

fn payload_too_large() -> Response {
    api_error(
        StatusCode::PAYLOAD_TOO_LARGE,
        "file exceeds maximum upload size",
    )
}

fn content_disposition(filename: &str) -> Result<HeaderValue, ApiError> {
    let fallback = filename
        .chars()
        .map(|character| {
            if character.is_ascii()
                && !character.is_ascii_control()
                && !matches!(character, '"' | '\\' | ';')
            {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let encoded = filename
        .as_bytes()
        .iter()
        .map(|byte| {
            if byte.is_ascii_alphanumeric()
                || matches!(
                    *byte,
                    b'!' | b'#'
                        | b'$'
                        | b'&'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
            {
                (*byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect::<String>();
    HeaderValue::from_str(&format!(
        "attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}"
    ))
    .map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid download filename",
        )
    })
}

fn files_error_response(error: FilesError) -> Response {
    let status = match error {
        FilesError::InvalidPath | FilesError::RootMutation => StatusCode::BAD_REQUEST,
        FilesError::NotFound => StatusCode::NOT_FOUND,
        FilesError::Conflict => StatusCode::CONFLICT,
        FilesError::NotDirectory | FilesError::NotFile | FilesError::UnsupportedEntry => {
            StatusCode::BAD_REQUEST
        },
        FilesError::Io(ref source) => {
            tracing::warn!(error = %source, "managed files storage operation failed");
            StatusCode::INTERNAL_SERVER_ERROR
        },
    };
    api_error(status, error.to_string())
}

fn api_error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(serde_json::json!({ "error": message.into() }))).into_response()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use {
        super::*,
        axum::{body::to_bytes, http::Request},
        moltis_gateway::{
            auth,
            auth::{AuthIdentity, AuthMethod},
            methods::MethodRegistry,
            services::GatewayServices,
            state::GatewayState,
        },
        std::sync::Arc,
        tower::ServiceExt,
    };

    fn identity(method: AuthMethod, scopes: &[&str]) -> AuthIdentity {
        AuthIdentity {
            method,
            scopes: scopes.iter().map(|scope| (*scope).to_owned()).collect(),
        }
    }

    #[test]
    fn upload_size_limit_is_checked_without_allocating_the_limit() {
        assert_eq!(
            checked_upload_size(MAX_FILE_UPLOAD_SIZE - 1, 1, MAX_FILE_UPLOAD_SIZE),
            Some(MAX_FILE_UPLOAD_SIZE)
        );
        assert_eq!(
            checked_upload_size(MAX_FILE_UPLOAD_SIZE, 1, MAX_FILE_UPLOAD_SIZE),
            None
        );
        assert_eq!(checked_upload_size(u64::MAX, 1, u64::MAX), None);
    }

    #[test]
    fn scopes_are_explicit_and_admin_is_accepted() {
        let read = identity(AuthMethod::ApiKey, &[scopes::READ]);
        let write = identity(AuthMethod::ApiKey, &[scopes::WRITE]);
        let admin = identity(AuthMethod::ApiKey, &[scopes::ADMIN]);
        let password = identity(AuthMethod::Password, &[]);

        assert!(require_files_access(Some(&read), FilesAccess::Read).is_ok());
        assert!(require_files_access(Some(&read), FilesAccess::Write).is_err());
        assert!(require_files_access(Some(&write), FilesAccess::Write).is_ok());
        assert!(require_files_access(Some(&write), FilesAccess::Read).is_err());
        assert!(require_files_access(Some(&admin), FilesAccess::Read).is_ok());
        assert!(require_files_access(Some(&admin), FilesAccess::Write).is_ok());
        assert!(require_files_access(Some(&password), FilesAccess::Write).is_ok());
    }

    #[test]
    fn content_disposition_is_attachment_only_and_unicode_safe() {
        let value = content_disposition("résumé \"final\".txt")
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        assert!(value.starts_with("attachment; filename="));
        assert!(value.contains("filename*=UTF-8''r%C3%A9sum%C3%A9%20%22final%22.txt"));
        assert!(!value.contains('\r'));
        assert!(!value.contains('\n'));
    }

    #[tokio::test]
    async fn router_streams_upload_list_and_download_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let files = Arc::new(LocalFilesService::new(temp.path()).unwrap());
        let gateway = GatewayState::new(
            auth::resolve_auth(None, None),
            GatewayServices::noop().with_files(files),
        );
        let methods = Arc::new(MethodRegistry::new());
        #[cfg(feature = "push-notifications")]
        let (_, state) = crate::server::build_gateway_base(gateway, methods, None, None);
        #[cfg(not(feature = "push-notifications"))]
        let (_, state) = crate::server::build_gateway_base(gateway, methods, None);
        let app = files_router().with_state(state);

        let upload = app
            .clone()
            .oneshot(
                Request::put("/content")
                    .header(
                        FILE_PATH_HEADER,
                        format!(
                            "{FILE_PATH_HEADER_PREFIX}{}",
                            URL_SAFE_NO_PAD.encode("résumé.txt".as_bytes())
                        ),
                    )
                    .body(Body::from("hello"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(upload.status(), StatusCode::CREATED);

        let list = app
            .clone()
            .oneshot(Request::get("/entries").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(list.status(), StatusCode::OK);
        let list_body = to_bytes(list.into_body(), 4096).await.unwrap();
        let listing: serde_json::Value = serde_json::from_slice(&list_body).unwrap();
        assert_eq!(listing["path"], "");
        assert_eq!(listing["entries"][0]["path"], "résumé.txt");
        assert_eq!(listing["entries"][0]["kind"], "file");
        assert_eq!(listing["entries"][0]["sizeBytes"], 5);
        assert!(listing["entries"][0].get("metadata").is_none());

        let download = app
            .oneshot(
                Request::get("/content")
                    .header(
                        FILE_PATH_HEADER,
                        format!(
                            "{FILE_PATH_HEADER_PREFIX}{}",
                            URL_SAFE_NO_PAD.encode("résumé.txt".as_bytes())
                        ),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(download.status(), StatusCode::OK);
        assert_eq!(download.headers()[header::CONTENT_LENGTH], "5");
        assert_eq!(
            download.headers()[header::CONTENT_TYPE],
            "application/octet-stream"
        );
        assert!(
            download.headers()[header::CONTENT_DISPOSITION]
                .to_str()
                .unwrap()
                .starts_with("attachment;")
        );
        assert_eq!(to_bytes(download.into_body(), 5).await.unwrap(), "hello");
    }

    #[test]
    fn content_length_limit_rejects_without_reading_a_body() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_LENGTH,
            HeaderValue::from_str(&(MAX_FILE_UPLOAD_SIZE + 1).to_string()).unwrap(),
        );
        assert!(content_length_exceeds_limit(&headers, MAX_FILE_UPLOAD_SIZE));
    }

    #[test]
    fn path_header_round_trips_unicode_as_ascii() {
        let mut headers = HeaderMap::new();
        let path = "健康/résumé.pdf";
        let encoded = URL_SAFE_NO_PAD.encode(path.as_bytes());
        headers.insert(
            FILE_PATH_HEADER,
            HeaderValue::from_str(&format!("{FILE_PATH_HEADER_PREFIX}{encoded}")).unwrap(),
        );
        assert_eq!(logical_path_header(&headers, true).unwrap(), path);
    }
}
