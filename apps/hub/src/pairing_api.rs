//! Administrative grant creation and unauthenticated, pinned-TLS redemption.
use crate::server::{ApiError, Shared, authenticate, persist_saved};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use neonmix_control::{Command, ControlError, Operation, Principal, Role};
use neonmix_identity::pairing::{Checked, Completion, Error as PairError, Invitation, Request};
use serde::Deserialize;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

// Book.open may evict expired grants and cancel removes them. Serialize these
// operations with redemption until its durable commit and Book.finish, while
// leaving Engine available to all audio/control workers during filesystem I/O.
static PAIRING_TRANSACTION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(super) enum Error {
    Control(ControlError),
    Pairing(PairError),
}
impl From<ControlError> for Error {
    fn from(e: ControlError) -> Self {
        Self::Control(e)
    }
}
impl From<PairError> for Error {
    fn from(e: PairError) -> Self {
        Self::Pairing(e)
    }
}
impl From<ApiError> for Error {
    fn from(e: ApiError) -> Self {
        Self::Control(e.0)
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let error = match self {
            Self::Control(error) => return ApiError(error).into_response(),
            Self::Pairing(error) => error,
        };
        let status = match error {
            PairError::Unauthorized => StatusCode::UNAUTHORIZED,
            PairError::Used => StatusCode::CONFLICT,
            PairError::Quota => StatusCode::TOO_MANY_REQUESTS,
            PairError::Invalid => StatusCode::BAD_REQUEST,
        };
        (status, Json(serde_json::json!({"error":error.to_string()}))).into_response()
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Open {
    ttl_seconds: u32,
}
fn admin(
    engine: &crate::server::Engine,
    headers: &HeaderMap,
) -> std::result::Result<Principal, ApiError> {
    let principal = authenticate(engine, headers)?;
    if engine
        .authority
        .current()
        .devices
        .get(&principal.device_id())
        .is_none_or(|d| d.role != Role::Admin)
    {
        return Err(ControlError::PermissionDenied.into());
    }
    Ok(principal)
}
pub(super) async fn identity(
    State(shared): State<Shared>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let engine = shared.lock().map_err(|_| ControlError::Busy)?;
    Ok(Json(
        serde_json::json!({"version":1,"hub_id":engine.authority.current().hub_id,"room_name":engine.room_name}),
    ))
}
pub(super) async fn open(
    State(shared): State<Shared>,
    headers: HeaderMap,
    Json(open): Json<Open>,
) -> std::result::Result<Json<Invitation>, Error> {
    let _transaction = PAIRING_TRANSACTION.lock().await;
    let mut engine = shared.lock().map_err(|_| ControlError::Busy)?;
    let issuer = admin(&engine, &headers)?;
    let (id, secret) = engine
        .pairings
        .open(issuer, open.ttl_seconds, Instant::now())?;
    let expires = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ControlError::Busy)?
        .as_secs()
        + u64::from(open.ttl_seconds);
    Ok(Json(Invitation {
        version: 1,
        hub_id: engine.authority.current().hub_id,
        room_name: engine.room_name.clone(),
        certificate: engine.certificate.clone(),
        invitation_id: id,
        secret,
        expires_unix_seconds: expires,
    }))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Cancel {
    pub(super) invitation_id: Uuid,
}
pub(super) async fn cancel(
    State(shared): State<Shared>,
    headers: HeaderMap,
    Json(cancel): Json<Cancel>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let _transaction = PAIRING_TRANSACTION.lock().await;
    let mut engine = shared.lock().map_err(|_| ControlError::Busy)?;
    admin(&engine, &headers)?;
    engine.pairings.cancel(cancel.invitation_id);
    Ok(Json(serde_json::json!({"cancelled":cancel.invitation_id})))
}
pub(super) async fn complete(
    State(shared): State<Shared>,
    headers: HeaderMap,
    Json(request): Json<Request>,
) -> std::result::Result<Json<Completion>, Error> {
    complete_with(shared, headers, request, persist_saved).await
}

pub(super) async fn complete_with(
    shared: Shared,
    headers: HeaderMap,
    request: Request,
    save: impl FnOnce(
        Option<&std::path::PathBuf>,
        &neonmix_control::PersistentState,
    ) -> std::result::Result<(), ControlError>
    + Send
    + 'static,
) -> std::result::Result<Json<Completion>, Error> {
    let transaction = PAIRING_TRANSACTION.lock().await;
    // The blocking task owns the entire transaction, including the Book gate.
    // Dropping an HTTP request/JoinHandle cannot abandon a frozen Authority or
    // allow cancellation/open to erase the grant after the file was committed.
    tokio::task::spawn_blocking(move || {
        let _transaction = transaction;
        complete_locked(shared, headers, request, save)
    })
    .await
    .map_err(|_| Error::Control(ControlError::Busy))?
}
fn complete_locked(
    shared: Shared,
    headers: HeaderMap,
    request: Request,
    save: impl FnOnce(
        Option<&std::path::PathBuf>,
        &neonmix_control::PersistentState,
    ) -> std::result::Result<(), ControlError>,
) -> std::result::Result<Json<Completion>, Error> {
    let mut engine = shared.lock().map_err(|_| ControlError::Busy)?;
    // A global fixed window bounds untrusted redemption work and memory.
    let now = Instant::now();
    if now.duration_since(engine.pair_window) >= std::time::Duration::from_secs(1) {
        engine.pair_window = now;
        engine.pair_attempts = 0;
    }
    if engine.pair_attempts >= 32 {
        return Err(PairError::Quota.into());
    }
    engine.pair_attempts += 1;
    let secret = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(PairError::Unauthorized)?;
    let issuer = match engine.pairings.check(secret, &request, now)? {
        Checked::Repeated(done) => {
            if engine
                .authority
                .current()
                .devices
                .get(&done.device_id)
                .is_none_or(|d| d.revoked)
            {
                return Err(PairError::Unauthorized.into());
            }
            return Ok(Json(done));
        }
        Checked::New(issuer) => issuer,
    };
    // Revalidate issuer and commit disk before publishing the registered identity.
    let path = engine.state_path.clone();
    let revision = engine.authority.current().revision;
    let prepared = engine.authority.prepare_transaction(
        issuer,
        Command {
            request_id: request.request_id,
            expected_revision: revision,
            operation: Operation::RegisterDevice {
                name: request.name.clone(),
                role: Role::Member,
                token_sha256: request.token_sha256.clone(),
            },
        },
    )?;
    let token = prepared.token();
    let saved = prepared.persistent();
    drop(engine);
    let persisted =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| save(path.as_ref(), &saved)))
            .unwrap_or(Err(ControlError::Busy));
    let mut engine = shared.lock().map_err(|_| ControlError::Busy)?;
    if let Err(error) = persisted {
        engine.authority.abort_transaction(token)?;
        return Err(error.into());
    }
    let receipt = engine.authority.commit_transaction(prepared)?;
    let done = Completion {
        hub_id: engine.authority.current().hub_id,
        device_id: receipt.device_id.ok_or(ControlError::Busy)?,
        revision: receipt.revision,
    };
    engine.pairings.finish(request, done.clone());
    Ok(Json(done))
}
