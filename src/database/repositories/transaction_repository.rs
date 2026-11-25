use crate::database::{
    models::{
        donation::DonationVerification,
        transaction::{BlockchainTransaction, RecordTransaction},
    },
    repositories::activity_repository,
};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

pub async fn record(
    pool: &PgPool,
    x: RecordTransaction,
) -> Result<BlockchainTransaction, sqlx::Error> {
    sqlx::query_as(r#"
            INSERT INTO blockchain_transactions(donation_id,withdrawal_id,network,tx_hash,from_address,to_address,block_number,confirmations)
            VALUES($1,$2,$3,$4,$5,$6,$7,COALESCE($8,0)) RETURNING *
        "#)
        .bind(x.donation_id)
        .bind(x.withdrawal_id)
        .bind(x.network)
        .bind(x.tx_hash.to_lowercase())
        .bind(x.from_address)
        .bind(x.to_address)
        .bind(x.block_number)
        .bind(x.confirmations)
        .fetch_one(pool).await
}

pub async fn confirm_donation(
    pool: &PgPool,
    donation_id: Uuid,
    donor_id: Uuid,
    network: &str,
    tx_hash: &str,
    from_address: &str,
    to_address: &str,
    block_number: i64,
    confirmations: i32,
    is_confirmed: bool,
) -> Result<DonationVerification, crate::error::AppError> {
    let mut tx = pool.begin().await?;
    let donation = sqlx::query_as::<
        _,
        (
            Uuid,
            Option<Uuid>,
            String,
            String,
            String,
            rust_decimal::Decimal,
            Option<rust_decimal::Decimal>,
            Option<String>,
            Option<rust_decimal::Decimal>,
            Option<String>,
            Option<chrono::DateTime<chrono::Utc>>,
        ),
    >(
        r#"
        SELECT d.campaign_id, d.donor_id, d.status, d.currency, c.title,
               d.amount, d.payment_amount, d.payment_currency, d.exchange_rate,
               d.exchange_rate_provider, d.exchange_rate_at
        FROM donations d
        JOIN campaigns c ON c.id=d.campaign_id
        WHERE d.id=$1
        FOR UPDATE OF d
    "#,
    )
    .bind(donation_id)
    .fetch_optional(&mut *tx)
    .await?;

    let Some((
        campaign_id,
        owner_id,
        status,
        currency,
        campaign_title,
        campaign_amount,
        payment_amount,
        payment_currency,
        exchange_rate,
        exchange_rate_provider,
        exchange_rate_at,
    )) = donation
    else {
        return Err(crate::error::AppError::NotFound);
    };
    if owner_id != Some(donor_id) {
        return Err(crate::error::AppError::Forbidden);
    }

    if status == "confirmed" {
        let existing_hash = sqlx::query_scalar::<_, String>(
            "SELECT tx_hash FROM blockchain_transactions WHERE donation_id=$1 AND verified=TRUE",
        )
        .bind(donation_id)
        .fetch_optional(&mut *tx)
        .await?;
        if existing_hash.as_deref() != Some(tx_hash) {
            return Err(crate::error::AppError::Conflict(
                "This contribution has already been confirmed with another transaction.".into(),
            ));
        }
        let existing_confirmations = sqlx::query_scalar::<_, i32>(
            "SELECT confirmations FROM blockchain_transactions WHERE donation_id=$1 AND tx_hash=$2",
        )
        .bind(donation_id)
        .bind(tx_hash)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok(DonationVerification {
            donation_id,
            status,
            transaction_hash: tx_hash.to_owned(),
            confirmations: existing_confirmations,
        });
    }

    let valid_eth_payment = if currency == "ETH" {
        payment_currency
            .as_deref()
            .is_none_or(|currency| currency == "ETH")
    } else {
        payment_currency.as_deref() == Some("ETH")
            && payment_amount.is_some()
            && exchange_rate.is_some()
            && exchange_rate_provider.is_some()
            && exchange_rate_at.is_some()
    };
    if status != "pending" || !valid_eth_payment {
        return Err(crate::error::AppError::Conflict(
            "This contribution cannot be confirmed.".into(),
        ));
    }
    let existing = sqlx::query_as::<_, (Uuid, String, String)>(
        "SELECT id, tx_hash, status FROM blockchain_transactions WHERE donation_id=$1 AND status IN ('pending','confirmed') ORDER BY created_at DESC LIMIT 1 FOR UPDATE",
    )
    .bind(donation_id)
    .fetch_optional(&mut *tx)
    .await?;
    let tx_status = if is_confirmed { "confirmed" } else { "pending" };
    if let Some((id, existing_hash, existing_status)) = existing {
        if existing_hash != tx_hash {
            return Err(crate::error::AppError::Conflict(
                "A different transaction is already being verified for this contribution.".into(),
            ));
        }
        if existing_status == "confirmed" {
            return Err(crate::error::AppError::Conflict(
                "This transaction is already associated with a confirmed contribution.".into(),
            ));
        }
        sqlx::query(
            "UPDATE blockchain_transactions SET block_number=$2, confirmations=$3, status=$4, verified=$5, confirmed_at=CASE WHEN $5 THEN now() ELSE NULL END WHERE id=$1",
        )
        .bind(id)
        .bind(block_number)
        .bind(confirmations)
        .bind(tx_status)
        .bind(is_confirmed)
        .execute(&mut *tx)
        .await?;
    } else {
        let already_used = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM blockchain_transactions WHERE network=$1 AND tx_hash=$2)",
        )
        .bind(network)
        .bind(tx_hash)
        .fetch_one(&mut *tx)
        .await?;
        if already_used {
            return Err(crate::error::AppError::Conflict(
                "This transaction is already associated with another record.".into(),
            ));
        }
        sqlx::query(
            r#"
            INSERT INTO blockchain_transactions(
                donation_id, network, tx_hash, from_address, to_address,
                block_number, confirmations, status, verified, confirmed_at
            )
            VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,CASE WHEN $9 THEN now() ELSE NULL END)
            "#,
        )
        .bind(donation_id)
        .bind(network)
        .bind(tx_hash)
        .bind(from_address)
        .bind(to_address)
        .bind(block_number)
        .bind(confirmations)
        .bind(tx_status)
        .bind(is_confirmed)
        .execute(&mut *tx)
        .await?;
    }

    if is_confirmed {
        sqlx::query("UPDATE donations SET status='confirmed', confirmed_at=now() WHERE id=$1")
            .bind(donation_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE campaigns c SET raised_amount=(SELECT COALESCE(SUM(d.amount),0) FROM donations d WHERE d.campaign_id=c.id AND d.status='confirmed'), updated_at=now() WHERE c.id=$1",
        )
        .bind(campaign_id)
        .execute(&mut *tx)
        .await?;
        let contribution_summary = if currency == "ETH" {
            format!("{} ETH", payment_amount.unwrap_or(campaign_amount))
        } else {
            format!(
                "{} ETH (recorded as {} {} at 1 ETH = {} {})",
                payment_amount.unwrap_or_default(),
                campaign_amount,
                currency,
                exchange_rate.unwrap_or_default(),
                currency
            )
        };
        if let Some(donor_id) = owner_id {
            activity_repository::notify_user(
                &mut tx,
                donor_id,
                "Contribution confirmed",
                &format!(
                    "Your {contribution_summary} contribution to {campaign_title} is confirmed{}.",
                    exchange_rate_provider
                        .as_deref()
                        .map(|provider| format!(" using the {provider} quote"))
                        .unwrap_or_default()
                ),
                Some("/my-donations"),
            )
            .await?;
        }
        let organizer_id =
            sqlx::query_scalar::<_, Uuid>("SELECT organizer_id FROM campaigns WHERE id=$1")
                .bind(campaign_id)
                .fetch_one(&mut *tx)
                .await?;
        activity_repository::notify_user(
            &mut tx,
            organizer_id,
            "Campaign contribution confirmed",
            &format!("A {contribution_summary} contribution to {campaign_title} is confirmed."),
            Some("/organizer/donations"),
        )
        .await?;
        activity_repository::record_audit(
            &mut tx,
            Some(donor_id),
            "donation.confirmed",
            "donation",
            Some(donation_id),
            json!({
                "network": network,
                "tx_hash": tx_hash,
                "confirmations": confirmations,
                "campaign_amount": campaign_amount,
                "campaign_currency": currency,
                "payment_amount": payment_amount,
                "payment_currency": payment_currency,
                "exchange_rate": exchange_rate,
                "exchange_rate_provider": exchange_rate_provider,
                "exchange_rate_at": exchange_rate_at
            }),
        )
        .await?;
    }

    tx.commit().await?;
    Ok(DonationVerification {
        donation_id,
        status: tx_status.into(),
        transaction_hash: tx_hash.to_owned(),
        confirmations,
    })
}
