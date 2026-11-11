use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use quil_types::proto::{
    global::{message_request, MessageBundle, MessageRequest},
    node::SendRequest,
    token::TokenOperation,
};
use tonic::transport::Channel;

use crate::domain::{
    WalletAction, WalletOperationSnapshot, WalletOperationStatus, WalletSubmissionOutcome,
};

use super::{
    super::node,
    super::prover,
    super::signing,
    operation_journal::{OperationJournal, OperationJournalError, OperationJournalPhase},
    ServiceError, Wallet,
};

pub(super) struct JournaledSubmission {
    pub outcome: WalletSubmissionOutcome,
    pub operation: WalletOperationSnapshot,
}

pub(super) struct PreparedSubmission {
    client: quil_types::proto::node::node_service_client::NodeServiceClient<Channel>,
    request: SendRequest,
}

pub(crate) fn prepare_submission(
    wallet: &Wallet,
    bytes: Vec<u8>,
) -> Result<(Vec<u8>, MessageRequest), ServiceError> {
    use quil_execution::token_engine::{
        TYPE_LATTICE_MINT, TYPE_LATTICE_MINT_CLAIM, TYPE_LATTICE_PENDING,
        TYPE_LATTICE_PENDING_CLAIM, TYPE_LATTICE_SHIELD, TYPE_LATTICE_TRANSACTION,
    };
    use quil_lattice_ct::confidential::{
        custom_mint::{self, CustomMint},
        mint::Mint,
        mint_claim::MintClaim,
        pending_claim::PendingClaim,
        pending_create::PendingCreate,
        transfer::Transfer,
    };

    let domain = quil_execution::token_intrinsic::wire::domain(&bytes)
        .map_err(|error| ServiceError::Operation(format!("Invalid operation framing: {error}")))?;
    if domain != wallet.application {
        return Err(ServiceError::Operation(
            "Operation belongs to another application".into(),
        ));
    }
    let kind = u32::from_be_bytes(
        bytes
            .get(..4)
            .ok_or_else(|| ServiceError::Operation("Operation is too short".into()))?
            .try_into()
            .map_err(|_| ServiceError::Operation("Invalid operation prefix".into()))?,
    );
    let custom_mint =
        kind == TYPE_LATTICE_MINT && bytes.get(4..12) == Some(custom_mint::VERSION.as_slice());
    let decoded = match kind {
        TYPE_LATTICE_MINT if custom_mint => {
            if domain == quil_execution::domains::QUIL_TOKEN {
                return Err(ServiceError::Operation(
                    "QUIL issuance must use reward mint".into(),
                ));
            }
            CustomMint::decode(&bytes, &wallet.network, &domain).map(|_| ())
        }
        TYPE_LATTICE_MINT => {
            if domain != quil_execution::domains::QUIL_TOKEN {
                return Err(ServiceError::Operation(
                    "Custom-token mint uses a different encoding".into(),
                ));
            }
            Mint::decode(&bytes, &wallet.network, &domain).map(|_| ())
        }
        TYPE_LATTICE_MINT_CLAIM => {
            if domain != quil_execution::domains::QUIL_TOKEN {
                return Err(ServiceError::Operation(
                    "Custom-token mint claim is not supported".into(),
                ));
            }
            MintClaim::decode(&bytes, &wallet.network, &domain).map(|_| ())
        }
        TYPE_LATTICE_SHIELD => quil_lattice_ct::confidential::shield::AnyShield::decode(
            &bytes,
            &wallet.network,
            &domain,
        )
        .map(|_| ()),
        TYPE_LATTICE_TRANSACTION => Transfer::decode(&bytes, &wallet.network, &domain).map(|_| ()),
        TYPE_LATTICE_PENDING => PendingCreate::decode(&bytes, &wallet.network, &domain).map(|_| ()),
        TYPE_LATTICE_PENDING_CLAIM => {
            PendingClaim::decode(&bytes, &wallet.network, &domain).map(|_| ())
        }
        quil_execution::token_intrinsic::constants::TYPE_LATTICE_SETTLEMENT => {
            if domain != quil_execution::domains::QUIL_TOKEN {
                return Err(ServiceError::Operation("Settlements spend QUIL".into()));
            }
            quil_lattice_ct::confidential::settlement::Settlement::decode(
                &bytes,
                &wallet.network,
                &domain,
            )
            .map(|_| ())
        }
        _ => {
            return Err(ServiceError::Operation(
                "Unsupported token operation".into(),
            ))
        }
    };
    decoded
        .map_err(|error| ServiceError::Operation(format!("Invalid token operation: {error:?}")))?;
    let destination = if kind == TYPE_LATTICE_MINT && !custom_mint {
        quil_execution::domains::GLOBAL
    } else {
        domain
    };
    Ok((
        destination.to_vec(),
        MessageRequest {
            request: Some(message_request::Request::TokenOperation(TokenOperation {
                canonical_bytes: bytes,
            })),
            ..Default::default()
        },
    ))
}

pub(super) async fn submit(
    config_dir: &Path,
    wallet: &Wallet,
    decision_client: &quil_types::proto::node::node_service_client::NodeServiceClient<Channel>,
    action: WalletAction,
    bytes: Vec<u8>,
) -> Result<JournaledSubmission, ServiceError> {
    let prepared = prepare(config_dir, wallet, bytes.clone()).await?;
    let journal = match OperationJournal::create(config_dir, action, wallet.application, &bytes) {
        Ok(journal) => journal,
        Err(OperationJournalError::AlreadyTracked { operation_id }) => {
            OperationJournal::recover(config_dir, decision_client).await?;
            let operation = OperationJournal::load_snapshot(config_dir, &operation_id)
                .map_err(journal_error)?;
            let outcome = tracked_outcome(&operation)?;
            return Ok(JournaledSubmission { outcome, operation });
        }
        Err(error) => return Err(journal_error(error)),
    };
    let outcome = match dispatch(prepared).await {
        Ok(outcome) => outcome,
        Err(error) => {
            if let Err(journal_error) =
                journal.record_submission(OperationJournalPhase::Rejected, Some(error.to_string()))
            {
                return Err(ServiceError::Operation(format!(
                    "{error}; additionally failed to persist the rejection: {journal_error}"
                )));
            }
            return Err(error);
        }
    };
    let (phase, detail) = match outcome {
        WalletSubmissionOutcome::Submitted => (
            OperationJournalPhase::Submitted,
            "Submitted to the node; awaiting a final GLOBAL decision".into(),
        ),
        WalletSubmissionOutcome::Unknown => (
            OperationJournalPhase::Unknown,
            "Submission result is unknown; the write will not be replayed".into(),
        ),
    };
    let operation = journal
        .record_submission(phase, Some(detail))
        .map_err(journal_error)?
        .into_snapshot()
        .map_err(journal_error)?;
    Ok(JournaledSubmission { outcome, operation })
}

fn journal_error(error: OperationJournalError) -> ServiceError {
    ServiceError::Wallet(error.to_string())
}

fn tracked_outcome(
    operation: &WalletOperationSnapshot,
) -> Result<WalletSubmissionOutcome, ServiceError> {
    match operation.phase {
        WalletOperationStatus::Rejected => Err(ServiceError::Operation(
            operation.detail.clone().unwrap_or_else(|| {
                format!(
                    "Tracked wallet operation {} was rejected",
                    operation.operation_id
                )
            }),
        )),
        WalletOperationStatus::Unknown => Ok(WalletSubmissionOutcome::Unknown),
        WalletOperationStatus::Submitting
        | WalletOperationStatus::Submitted
        | WalletOperationStatus::Committed => Ok(WalletSubmissionOutcome::Submitted),
    }
}

pub(super) async fn prepare(
    config_dir: &Path,
    wallet: &Wallet,
    bytes: Vec<u8>,
) -> Result<PreparedSubmission, ServiceError> {
    let config = quil_config::load_config(config_dir)
        .map_err(|error| ServiceError::Config(error.to_string()))?;
    let manager = prover::build_key_manager(&config, config_dir)?;
    let (domain, request) = prepare_submission(wallet, bytes)?;
    let bundle = MessageBundle {
        requests: vec![request],
        timestamp: now_millis(),
    };
    let authentication = signing::sign_bundle(&manager, &domain, &bundle)
        .map_err(|error| ServiceError::Operation(error.to_string()))?;
    let address = node::grpc_address(config.effective_listen_grpc_multiaddr())?;
    let client = node::connect_client(&address).await?;
    Ok(PreparedSubmission {
        client,
        request: SendRequest {
            domain,
            request: Some(bundle),
            authentication,
            delivery_data: Vec::new(),
        },
    })
}

pub(super) async fn dispatch(
    prepared: PreparedSubmission,
) -> Result<WalletSubmissionOutcome, ServiceError> {
    let mut client = prepared.client;
    let result = tokio::time::timeout(
        node::RPC_TIMEOUT,
        client.send(tonic::Request::new(prepared.request)),
    )
    .await;
    match result {
        Err(_) => Ok(WalletSubmissionOutcome::Unknown),
        Ok(result) => classify_send_result(result.map(|_| ())),
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn classify_send_result(
    result: Result<(), tonic::Status>,
) -> Result<WalletSubmissionOutcome, ServiceError> {
    match result {
        Ok(()) => Ok(WalletSubmissionOutcome::Submitted),
        Err(status)
            if matches!(
                status.code(),
                tonic::Code::InvalidArgument
                    | tonic::Code::Unauthenticated
                    | tonic::Code::PermissionDenied
                    | tonic::Code::FailedPrecondition
                    | tonic::Code::Unimplemented
            ) =>
        {
            Err(ServiceError::Operation(format!(
                "Node rejected submission: {}",
                status.message()
            )))
        }
        // Once `send` has been called, no status is a reliable proof that the bundle was not
        // queued for node-side delivery/retry. Treat every non-OK response as ambiguous and
        // recover by reading chain state only.
        Err(_) => Ok(WalletSubmissionOutcome::Unknown),
    }
}

#[cfg(test)]
mod tests {
    use super::{classify_send_result, tracked_outcome};
    use crate::domain::WalletSubmissionOutcome;
    use crate::domain::{WalletAction, WalletOperationSnapshot, WalletOperationStatus};

    #[test]
    fn post_dispatch_transport_loss_is_unknown_not_failed() {
        for code in [
            tonic::Code::Cancelled,
            tonic::Code::Unknown,
            tonic::Code::DeadlineExceeded,
            tonic::Code::Unavailable,
        ] {
            assert_eq!(
                classify_send_result(Err(tonic::Status::new(code, "lost"))).unwrap(),
                WalletSubmissionOutcome::Unknown
            );
        }
    }

    #[test]
    fn explicit_rpc_rejections_preserve_the_server_error() {
        assert_eq!(
            classify_send_result(Ok(())).unwrap(),
            WalletSubmissionOutcome::Submitted
        );
        for code in [
            tonic::Code::InvalidArgument,
            tonic::Code::Unauthenticated,
            tonic::Code::PermissionDenied,
            tonic::Code::FailedPrecondition,
            tonic::Code::Unimplemented,
        ] {
            let error =
                classify_send_result(Err(tonic::Status::new(code, "server rejected operation")))
                    .unwrap_err()
                    .to_string();
            assert!(error.contains("server rejected operation"), "{error}");
        }
    }

    #[test]
    fn ambiguous_transport_or_delivery_errors_remain_unknown() {
        for code in [
            tonic::Code::Cancelled,
            tonic::Code::Unknown,
            tonic::Code::DeadlineExceeded,
            tonic::Code::ResourceExhausted,
            tonic::Code::Aborted,
            tonic::Code::Internal,
            tonic::Code::Unavailable,
            tonic::Code::DataLoss,
            tonic::Code::AlreadyExists,
        ] {
            assert_eq!(
                classify_send_result(Err(tonic::Status::new(code, "delivery uncertain"))).unwrap(),
                WalletSubmissionOutcome::Unknown
            );
        }
    }

    #[test]
    fn a_tracked_rejection_returns_its_real_error_instead_of_unknown() {
        let operation = WalletOperationSnapshot {
            operation_id: "11".repeat(32),
            action: WalletAction::Transfer,
            application_hex: "22".repeat(32),
            phase: WalletOperationStatus::Rejected,
            submitted_unix_ms: Some(42),
            detail: Some("Node rejected submission: invalid proof".into()),
            console_version: "0.1.0".into(),
            build_commit: "test".into(),
        };

        let error = tracked_outcome(&operation).unwrap_err().to_string();

        assert!(error.contains("invalid proof"));
        assert!(!error.contains("unknown"));
    }
}
