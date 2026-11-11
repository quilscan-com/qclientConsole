use quil_types::proto::node::{
    node_service_client::NodeServiceClient, GetTokenFeeQuoteRequest, GetTokenFeeQuoteResponse,
};
use tonic::transport::Channel;

use super::super::{node, ServiceError};

pub(super) const AUTOMATIC_FEE_HEADROOM_PERCENT: u128 = 10;

pub(super) fn with_fee_headroom(charge: u128) -> Result<u128, ServiceError> {
    let headroom = charge
        .checked_mul(AUTOMATIC_FEE_HEADROOM_PERCENT)
        .map(|scaled| scaled.div_ceil(100))
        .ok_or_else(|| ServiceError::Wallet("Fee exceeds the token amount range".into()))?;
    charge
        .checked_add(headroom)
        .ok_or_else(|| ServiceError::Wallet("Fee exceeds the token amount range".into()))
}

pub(super) fn enforce_max_fee(amount: u128, max_fee: u128) -> Result<u128, ServiceError> {
    if amount > max_fee {
        return Err(ServiceError::Wallet(format!(
            "The automatic fee of {amount} base units exceeds the approved maximum of {max_fee} base units"
        )));
    }
    Ok(amount)
}

pub(crate) fn growth_for_shape(
    network: &[u8; 32],
    application: &[u8; 32],
    shape: quil_execution::token_intrinsic::cost::Shape,
) -> Result<u64, ServiceError> {
    let context = quil_lattice_ct::confidential::transfer::parameter_context(network, application);
    quil_execution::token_intrinsic::cost::shape_growth(&context, shape)
        .map_err(|error| ServiceError::Wallet(format!("Cannot price wallet operation: {error}")))
}

pub(super) fn checked_quil_fee_quote(
    quote: &GetTokenFeeQuoteResponse,
    network: &[u8; 32],
    cost: u64,
) -> Result<u128, ServiceError> {
    if quote.network.as_slice() != network.as_slice()
        || quote.application.as_slice() != quil_execution::domains::QUIL_TOKEN.as_slice()
        || quote.max_payload_bytes != cost
        || cost == 0
        || cost >= 1 << 20
    {
        return Err(ServiceError::Wallet(
            "Fee quote context does not match the request".into(),
        ));
    }
    let budget =
        u128::from_be_bytes(quote.fee_budget.as_slice().try_into().map_err(|_| {
            ServiceError::Wallet("Fee quote amount must be exactly 16 bytes".into())
        })?);
    let selector = quil_execution::pricing::network_selector(network)
        .ok_or_else(|| ServiceError::Wallet("Invalid network identifier in fee quote".into()))?;
    let expected = quil_execution::pricing::fee_budget_for_payload_limit(
        selector,
        quote.difficulty,
        quote.world_state_bytes,
        cost,
        quote.fee_multiplier_vote,
    )
    .map_err(|error| ServiceError::Wallet(format!("Invalid fee quote: {error}")))?;
    if num_bigint::BigInt::from(budget) != expected {
        return Err(ServiceError::Wallet(
            "Fee quote arithmetic does not match its pricing inputs".into(),
        ));
    }
    if !quote.global_execution || quote.fee_multiplier_vote != 1 {
        return Err(ServiceError::Wallet(
            "QUIL fee quote is not priced at the global venue".into(),
        ));
    }
    let cost_big = num_bigint::BigInt::from(cost);
    let required = quil_execution::pricing::fee_multiplier_for_cost(
        selector,
        quote.difficulty,
        quote.world_state_bytes,
        &cost_big,
        quote.fee_multiplier_vote,
    )
    .map_err(|error| ServiceError::Wallet(format!("Invalid fee quote: {error}")))?
        * &cost_big;
    if required > expected {
        return Err(ServiceError::Wallet(
            "Fee quote budget does not cover the exact charge".into(),
        ));
    }
    required
        .to_string()
        .parse::<u128>()
        .map_err(|_| ServiceError::Wallet("Fee exceeds the token amount range".into()))
}

pub(super) fn check_submission_fee_quote(
    quote: &GetTokenFeeQuoteResponse,
    network: &[u8; 32],
    cost: u64,
    paid: u128,
) -> Result<(), ServiceError> {
    let required = checked_quil_fee_quote(quote, network, cost)?;
    if paid < required {
        return Err(ServiceError::Wallet(format!(
            "Fee changed while preparing the transaction: approved {paid}, current quote requires {required} base units"
        )));
    }
    Ok(())
}

pub(crate) async fn quoted_quil_fee_for_cost(
    client: &mut NodeServiceClient<Channel>,
    network: &[u8; 32],
    explicit: Option<u128>,
    cost: u64,
    global_venue: bool,
    max_fee: u128,
) -> Result<u128, ServiceError> {
    if let Some(fee) = explicit {
        return Ok(fee);
    }
    let quote = tokio::time::timeout(
        node::RPC_TIMEOUT,
        client.get_token_fee_quote(GetTokenFeeQuoteRequest {
            application: quil_execution::domains::QUIL_TOKEN.to_vec(),
            max_payload_bytes: cost,
            global_venue,
        }),
    )
    .await
    .map_err(|_| ServiceError::Timeout)?
    .map_err(|error| {
        ServiceError::Wallet(format!(
            "Fee estimate is unavailable from the configured node: {error}"
        ))
    })?
    .into_inner();
    if global_venue && !quote.global_execution {
        return Err(ServiceError::Wallet(
            "Global wallet operation was not quoted for global execution".into(),
        ));
    }
    let charge = checked_quil_fee_quote(&quote, network, cost)?;
    enforce_max_fee(with_fee_headroom(charge)?, max_fee)
}

pub(crate) async fn refresh_submission_fee(
    client: &mut NodeServiceClient<Channel>,
    network: &[u8; 32],
    bytes: &[u8],
) -> Result<(), ServiceError> {
    use quil_execution::token_intrinsic::{cost, wire};

    let application = wire::domain(bytes)
        .map_err(|error| ServiceError::Wallet(format!("Invalid operation framing: {error}")))?;
    let kind = u32::from_be_bytes(
        bytes
            .get(..4)
            .ok_or_else(|| ServiceError::Wallet("Operation is too short".into()))?
            .try_into()
            .map_err(|_| ServiceError::Wallet("Invalid operation prefix".into()))?,
    );
    if application != quil_execution::domains::QUIL_TOKEN
        || !matches!(kind, 0x0512 | 0x0513 | 0x0514 | 0x0515 | 0x0516 | 0x0518)
    {
        return Ok(());
    }
    let growth = cost::state_growth(bytes)
        .map_err(|error| ServiceError::Wallet(format!("Cannot price operation: {error}")))?;
    let quote = match tokio::time::timeout(
        node::RPC_TIMEOUT,
        client.get_token_fee_quote(GetTokenFeeQuoteRequest {
            application: application.to_vec(),
            max_payload_bytes: growth,
            global_venue: kind == quil_execution::token_intrinsic::constants::TYPE_LATTICE_MINT,
        }),
    )
    .await
    {
        Err(_) => return Ok(()),
        Ok(Err(error))
            if matches!(
                error.code(),
                tonic::Code::Unavailable | tonic::Code::Unimplemented
            ) =>
        {
            return Ok(())
        }
        Ok(Err(error)) => return Err(ServiceError::Rpc(error.to_string())),
        Ok(Ok(response)) => response.into_inner(),
    };
    let paid = wire::fee(bytes)
        .map_err(|error| ServiceError::Wallet(format!("Cannot read operation fee: {error}")))?;
    check_submission_fee_quote(&quote, network, growth, paid)
}

#[cfg(test)]
mod tests {
    use super::{
        check_submission_fee_quote, checked_quil_fee_quote, enforce_max_fee, with_fee_headroom,
    };

    #[test]
    fn fee_headroom_matches_qclient_rounding() {
        assert_eq!(with_fee_headroom(0).unwrap(), 0);
        assert_eq!(with_fee_headroom(1).unwrap(), 2);
        assert_eq!(with_fee_headroom(3_199_680).unwrap(), 3_519_648);
        assert!(with_fee_headroom(u128::MAX).is_err());
    }

    #[test]
    fn fee_limit_rejects_quotes_above_the_approved_maximum() {
        assert!(enforce_max_fee(121, 120).is_err());
        assert_eq!(enforce_max_fee(120, 120).unwrap(), 120);
    }

    #[test]
    fn quote_context_and_arithmetic_are_verified_locally() {
        let quote = quil_types::proto::node::GetTokenFeeQuoteResponse {
            application: quil_execution::domains::QUIL_TOKEN.to_vec(),
            network: vec![0; 32],
            observed_frame: 42,
            global_execution: true,
            difficulty: 50_000,
            world_state_bytes: 0,
            fee_multiplier_vote: 1,
            max_payload_bytes: 64,
            fee_budget: 127u128.to_be_bytes().to_vec(),
        };
        assert_eq!(checked_quil_fee_quote(&quote, &[0; 32], 64).unwrap(), 64);
        assert!(check_submission_fee_quote(&quote, &[0; 32], 64, 64).is_ok());
        assert!(check_submission_fee_quote(&quote, &[0; 32], 64, 63).is_err());

        let mut wrong_venue = quote.clone();
        wrong_venue.global_execution = false;
        assert!(checked_quil_fee_quote(&wrong_venue, &[0; 32], 64).is_err());

        let mut wrong_budget = quote;
        wrong_budget.fee_budget = 126u128.to_be_bytes().to_vec();
        assert!(checked_quil_fee_quote(&wrong_budget, &[0; 32], 64).is_err());
    }
}
