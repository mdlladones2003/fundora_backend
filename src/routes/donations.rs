use crate::{
    app::AppState,
    auth::{AuthUser, has_role},
    database::{
        models::donation::{DonationReceipt, DonationVerification},
        repositories::transaction_repository,
    },
    error::AppError,
};
use alloy::{
    primitives::{Address, B256, U256},
    providers::{Provider, ProviderBuilder},
    sol,
    sol_types::SolEvent,
};
use axum::{
    Json,
    extract::{Extension, Path, State},
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use uuid::Uuid;

sol! {
    interface FundoraDonationContract {
        event DonationReceived(
            bytes32 indexed donationId,
            bytes32 indexed campaignId,
            address indexed donor,
            address recipient,
            uint256 amount
        );
    }
}

#[derive(Debug, Deserialize)]
pub struct VerifyDonationRequest {
    pub transaction_hash: String,
}

#[derive(Debug, Serialize)]
pub struct PaymentConfig {
    pub chain_id: u64,
    pub contract_address: String,
    pub minimum_confirmations: u64,
}

fn uuid_as_b256(id: Uuid) -> B256 {
    let mut bytes = [0; 32];
    bytes[..16].copy_from_slice(id.as_bytes());
    B256::from(bytes)
}

fn eth_to_wei(amount: Decimal) -> Result<U256, AppError> {
    let amount = amount.normalize().to_string();
    let (whole, fraction) = amount.split_once('.').unwrap_or((&amount, ""));
    let fraction = format!("{fraction:0<18}");
    U256::from_str_radix(&format!("{whole}{fraction}"), 10)
        .map_err(|_| AppError::BadRequest("Contribution amount is too large.".into()))
}

pub async fn payment_config(
    State(state): State<AppState>,
) -> Result<Json<PaymentConfig>, AppError> {
    let contract_address =
        state
            .config
            .evm_donation_contract
            .as_deref()
            .ok_or(AppError::ServiceUnavailable(
                "EVM donations are not configured.",
            ))?;
    let contract_address = Address::from_str(contract_address).map_err(|_| {
        AppError::ServiceUnavailable("EVM_DONATION_CONTRACT is not a valid address.")
    })?;
    if contract_address.is_zero() || state.config.evm_min_confirmations == 0 {
        return Err(AppError::ServiceUnavailable(
            "EVM donation configuration is invalid.",
        ));
    }
    if state.config.evm_chain_id != 11_155_111 {
        return Err(AppError::ServiceUnavailable(
            "EVM donations are currently configured for Sepolia only.",
        ));
    }
    let rpc_url = state
        .config
        .evm_rpc_url
        .as_deref()
        .ok_or(AppError::ServiceUnavailable(
            "EVM_RPC_URL is not configured.",
        ))?
        .parse::<reqwest::Url>()
        .map_err(|_| AppError::ServiceUnavailable("EVM_RPC_URL is not a valid URL."))?;
    let provider = ProviderBuilder::new().connect_http(rpc_url);
    let actual_chain_id = provider.get_chain_id().await.map_err(|error| {
        tracing::error!(%error, "Could not read EVM chain ID");
        AppError::ServiceUnavailable("The EVM RPC is temporarily unavailable.")
    })?;
    if actual_chain_id != state.config.evm_chain_id || actual_chain_id != 11_155_111 {
        return Err(AppError::ServiceUnavailable(
            "The configured EVM RPC is not connected to Sepolia.",
        ));
    }
    let code = provider
        .get_code_at(contract_address)
        .await
        .map_err(|error| {
            tracing::error!(%error, "Could not read donation contract code");
            AppError::ServiceUnavailable("The EVM RPC is temporarily unavailable.")
        })?;
    if code.is_empty() {
        return Err(AppError::ServiceUnavailable(
            "The configured donation contract is not deployed on Sepolia.",
        ));
    }
    Ok(Json(PaymentConfig {
        chain_id: state.config.evm_chain_id,
        contract_address: contract_address.to_string(),
        minimum_confirmations: state.config.evm_min_confirmations,
    }))
}

pub async fn verify(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
    Json(body): Json<VerifyDonationRequest>,
) -> Result<Json<DonationVerification>, AppError> {
    if !has_role(&user, &["donor"]) {
        return Err(AppError::Forbidden);
    }

    let tx_hash = B256::from_str(body.transaction_hash.trim())
        .map_err(|_| AppError::BadRequest("A valid transaction hash is required.".into()))?;
    let tx_hash_string = tx_hash.to_string();
    let donation = sqlx::query_as::<
        _,
        (
            Decimal,
            String,
            String,
            Option<String>,
            Option<String>,
            Uuid,
            Option<Decimal>,
            Option<String>,
        ),
    >(
        r#"
        SELECT d.amount, d.currency, d.status, d.donor_wallet_address,
               d.recipient_wallet_address, d.campaign_id,
               d.payment_amount, d.payment_currency
        FROM donations d
        WHERE d.id=$1 AND d.donor_id=$2
        "#,
    )
    .bind(id)
    .bind(user.id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    let (
        amount,
        currency,
        donation_status,
        donor_wallet,
        recipient_wallet,
        campaign_id,
        payment_amount,
        payment_currency,
    ) = donation;
    if donation_status == "confirmed" {
        let confirmations = sqlx::query_scalar::<_, i32>(
            "SELECT confirmations FROM blockchain_transactions WHERE donation_id=$1 AND tx_hash=$2 AND verified=TRUE",
        )
        .bind(id)
        .bind(&tx_hash_string)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::Conflict(
            "This contribution was confirmed using a different transaction.".into(),
        ))?;
        return Ok(Json(DonationVerification {
            donation_id: id,
            status: "confirmed".into(),
            transaction_hash: tx_hash_string,
            confirmations,
        }));
    }
    if donation_status != "pending" {
        return Err(AppError::Conflict(
            "This contribution is not awaiting payment verification.".into(),
        ));
    }
    let expected_eth_amount = match payment_amount {
        Some(amount) if payment_currency.as_deref() == Some("ETH") => amount,
        None if currency == "ETH" => amount,
        _ => {
            return Err(AppError::Conflict(
                "This contribution does not have a valid ETH payment quote.".into(),
            ));
        }
    };

    let donor_wallet = Address::from_str(donor_wallet.as_deref().ok_or(AppError::Conflict(
        "The donor wallet was not recorded.".into(),
    ))?)
    .map_err(|_| AppError::Internal)?;
    let recipient_wallet = Address::from_str(recipient_wallet.as_deref().ok_or(
        AppError::Conflict("The organizer wallet was not recorded.".into()),
    )?)
    .map_err(|_| AppError::Internal)?;
    let expected_amount = eth_to_wei(expected_eth_amount)?;
    let contract_address = Address::from_str(state.config.evm_donation_contract.as_deref().ok_or(
        AppError::ServiceUnavailable("EVM donations are not configured."),
    )?)
    .map_err(|_| AppError::ServiceUnavailable("EVM_DONATION_CONTRACT is not a valid address."))?;
    if contract_address.is_zero() || state.config.evm_min_confirmations == 0 {
        return Err(AppError::ServiceUnavailable(
            "EVM donation configuration is invalid.",
        ));
    }
    let rpc_url = state
        .config
        .evm_rpc_url
        .as_deref()
        .ok_or(AppError::ServiceUnavailable(
            "EVM_RPC_URL is not configured.",
        ))?
        .parse::<reqwest::Url>()
        .map_err(|_| AppError::ServiceUnavailable("EVM_RPC_URL is not a valid URL."))?;
    let provider = ProviderBuilder::new().connect_http(rpc_url);
    let actual_chain_id = provider.get_chain_id().await.map_err(|error| {
        tracing::error!(%error, "Could not read EVM chain ID");
        AppError::ServiceUnavailable("The EVM RPC is temporarily unavailable.")
    })?;
    if actual_chain_id != state.config.evm_chain_id || actual_chain_id != 11_155_111 {
        return Err(AppError::ServiceUnavailable(
            "The configured EVM RPC is not connected to Sepolia.",
        ));
    }

    let Some(receipt) = provider
        .get_transaction_receipt(tx_hash)
        .await
        .map_err(|error| {
            tracing::error!(%error, "Could not read EVM transaction receipt");
            AppError::ServiceUnavailable("The EVM RPC is temporarily unavailable.")
        })?
    else {
        return Ok(Json(DonationVerification {
            donation_id: id,
            status: "pending".into(),
            transaction_hash: tx_hash_string,
            confirmations: 0,
        }));
    };

    if !receipt.status() {
        return Err(AppError::Conflict(
            "The on-chain donation transaction failed.".into(),
        ));
    }
    if receipt.to != Some(contract_address) || receipt.from != donor_wallet {
        return Err(AppError::BadRequest(
            "The transaction sender or contract does not match this contribution.".into(),
        ));
    }
    let has_matching_event = receipt
        .logs()
        .iter()
        .filter(|log| log.address() == contract_address)
        .filter_map(|log| {
            FundoraDonationContract::DonationReceived::decode_log(&log.inner)
                .ok()
                .map(|decoded| decoded.data)
        })
        .any(|_event| {
            _event.donationId == uuid_as_b256(id)
                && _event.campaignId == uuid_as_b256(campaign_id)
                && _event.donor == donor_wallet
                && _event.recipient == recipient_wallet
                && _event.amount == expected_amount
        });
    if !has_matching_event {
        return Err(AppError::BadRequest(
            "The transaction does not contain a matching Fundora donation event.".into(),
        ));
    }
    let block_number = receipt.block_number.ok_or(AppError::ServiceUnavailable(
        "The mined transaction has no block number.",
    ))?;
    let current_block = provider.get_block_number().await.map_err(|error| {
        tracing::error!(%error, "Could not read EVM block height");
        AppError::ServiceUnavailable("The EVM RPC is temporarily unavailable.")
    })?;
    let confirmations = current_block.saturating_sub(block_number).saturating_add(1);
    let confirmation_count = i32::try_from(confirmations).unwrap_or(i32::MAX);
    let is_confirmed = confirmations >= state.config.evm_min_confirmations;
    transaction_repository::confirm_donation(
        &state.pool,
        id,
        user.id,
        &format!("eip155:{actual_chain_id}"),
        &tx_hash_string,
        &donor_wallet.to_string(),
        &contract_address.to_string(),
        i64::try_from(block_number).map_err(|_| AppError::Internal)?,
        confirmation_count,
        is_confirmed,
    )
    .await
    .map(Json)
}

pub async fn list_mine(
    State(s): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<Vec<crate::database::models::donation::DonationHistory>>, AppError> {
    if !has_role(&user, &["donor"]) {
        return Err(AppError::Forbidden);
    }
    let donations = sqlx::query_as::<_, crate::database::models::donation::DonationHistory>(
        r#"
        SELECT d.*, p.method AS payment_method, p.provider AS payment_provider,
               p.session_id AS payment_reference, c.title AS campaign_title,
               t.tx_hash AS transaction_hash, t.network AS transaction_network,
               t.status AS transaction_status,
               t.verified AS transaction_verified
        FROM donations d
        JOIN campaigns c ON c.id=d.campaign_id
        LEFT JOIN payment_attempts p ON p.donation_id=d.id
        LEFT JOIN LATERAL (
            SELECT tx_hash, network, status, verified
            FROM blockchain_transactions
            WHERE donation_id=d.id ORDER BY created_at DESC LIMIT 1
        ) t ON TRUE
        WHERE d.donor_id=$1
        ORDER BY d.created_at DESC
    "#,
    )
    .bind(user.id)
    .fetch_all(&s.pool)
    .await?;

    Ok(Json(donations))
}

pub async fn get_mine(
    State(s): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<DonationReceipt>, AppError> {
    if !has_role(&user, &["donor"]) {
        return Err(AppError::Forbidden);
    }
    let receipt = sqlx::query_as::<_, DonationReceipt>(
        r#"
        SELECT d.*, p.method AS payment_method, p.provider AS payment_provider,
               p.session_id AS payment_reference, c.title AS campaign_title,
               t.tx_hash AS transaction_hash, t.network AS transaction_network,
               t.status AS transaction_status, t.verified AS transaction_verified,
               t.block_number, t.confirmations,
               t.confirmed_at AS transaction_confirmed_at
        FROM donations d
        JOIN campaigns c ON c.id = d.campaign_id
        LEFT JOIN payment_attempts p ON p.donation_id = d.id
        LEFT JOIN LATERAL (
            SELECT tx_hash, network, status, verified, block_number, confirmations, confirmed_at
            FROM blockchain_transactions
            WHERE donation_id = d.id
            ORDER BY created_at DESC
            LIMIT 1
        ) t ON TRUE
        WHERE d.id = $1 AND d.donor_id = $2
        "#,
    )
    .bind(id)
    .bind(user.id)
    .fetch_optional(&s.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(Json(receipt))
}

#[cfg(test)]
mod tests {
    use super::{eth_to_wei, uuid_as_b256};
    use rust_decimal::Decimal;
    use std::str::FromStr;
    use uuid::Uuid;

    #[test]
    fn converts_eth_to_wei_without_float_rounding() {
        assert_eq!(
            eth_to_wei(Decimal::from_str("0.000000000000000001").unwrap())
                .unwrap()
                .to_string(),
            "1"
        );
        assert_eq!(
            eth_to_wei(Decimal::from_str("1.25").unwrap())
                .unwrap()
                .to_string(),
            "1250000000000000000"
        );
    }

    #[test]
    fn encodes_uuid_as_the_contract_bytes32_value() {
        let id = Uuid::parse_str("12345678-1234-5678-90ab-cdef12345678").unwrap();
        assert_eq!(
            uuid_as_b256(id).to_string(),
            "0x123456781234567890abcdef1234567800000000000000000000000000000000"
        );
    }
}
