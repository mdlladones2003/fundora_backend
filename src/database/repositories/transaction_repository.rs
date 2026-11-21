use crate::database::{
    models::transaction::{BlockchainTransaction, RecordTransaction},
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

pub async fn verify(
    pool: &PgPool,
    id: Uuid,
    confirmations: i32,
    block: i64,
) -> Result<Option<BlockchainTransaction>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query_as::<_, BlockchainTransaction>(
        r#"
        UPDATE blockchain_transactions
        SET confirmations=$2, block_number=$3, status='confirmed', verified=true, confirmed_at=now()
        WHERE id=$1 AND status='pending' AND verified=FALSE RETURNING *
    "#,
    )
    .bind(id)
    .bind(confirmations)
    .bind(block)
    .fetch_optional(&mut *tx)
    .await?;

    if let Some(ref t) = row {
        if let Some(donation_id) = t.donation_id {
            sqlx::query("UPDATE donations SET status='confirmed', confirmed_at=now() WHERE id=$1")
                .bind(donation_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE campaigns c SET raised_amount=(SELECT COALESCE(SUM(d.amount),0) FROM donations d WHERE d.campaign_id=c.id AND d.status='confirmed'), updated_at=now() WHERE c.id=(SELECT campaign_id FROM donations WHERE id=$1)")
                .bind(donation_id).execute(&mut *tx).await?;
            let (donor_id, organizer_id, campaign_title) =
                sqlx::query_as::<_, (Option<Uuid>, Uuid, String)>(
                    r#"
                SELECT d.donor_id, c.organizer_id, c.title
                FROM donations d JOIN campaigns c ON c.id = d.campaign_id
                WHERE d.id = $1
                "#,
                )
                .bind(donation_id)
                .fetch_one(&mut *tx)
                .await?;
            if let Some(donor_id) = donor_id {
                activity_repository::notify_user(
                    &mut tx,
                    donor_id,
                    "Contribution transaction confirmed",
                    &format!("A blockchain confirmation was recorded for {campaign_title}."),
                    Some("/donations"),
                )
                .await?;
            }
            activity_repository::notify_user(
                &mut tx,
                organizer_id,
                "Campaign contribution confirmed",
                &format!(
                    "A contribution to {campaign_title} has a recorded blockchain confirmation."
                ),
                Some("/dashboard/organizer"),
            )
            .await?;
        }
        activity_repository::record_audit(
            &mut tx,
            None,
            "transaction.confirmed",
            "blockchain_transaction",
            Some(t.id),
            json!({"network": t.network, "tx_hash": t.tx_hash, "confirmations": confirmations, "block_number": block}),
        )
        .await?;
    } else {
        let already_confirmed = sqlx::query_as::<_, BlockchainTransaction>(
            "SELECT * FROM blockchain_transactions WHERE id = $1 AND verified = TRUE",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok(already_confirmed);
    }

    tx.commit().await?;
    Ok(row)
}
