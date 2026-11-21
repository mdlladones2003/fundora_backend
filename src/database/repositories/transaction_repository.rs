use sqlx::PgPool;
use uuid::Uuid;
use crate::database::models::transaction::{BlockchainTransaction, RecordTransaction};

pub async fn record(pool: &PgPool, x: RecordTransaction) -> Result<BlockchainTransaction, sqlx::Error> {
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

pub async fn verify(pool: &PgPool, id: Uuid, confirmations: i32, block: i64) -> Result<Option<BlockchainTransaction>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query_as::<_, BlockchainTransaction>(r#"
        UPDATE blockchain_transactions
        SET confirmations=$2, block_number=$3, status='confirmed', verified=true, confirmed_at=now()
        WHERE id=$1 RETURNING *
    "#)
    .bind(id)
    .bind(confirmations)
    .bind(block)
    .fetch_optional(&mut *tx).await?;
    
    if let Some(ref t) = row {
        if let Some(donation_id) = t.donation_id {
            sqlx::query("UPDATE donations SET status='confirmed', confirmed_at=now() WHERE id=$1")
                .bind(donation_id).execute(&mut *tx).await?;
            sqlx::query("UPDATE campaigns c SET raised_amount=(SELECT COALESCE(SUM(d.amount),0) FROM donations d WHERE d.campaign_id=c.id AND d.status='confirmed'), updated_at=now() WHERE c.id=(SELECT campaign_id FROM donations WHERE id=$1)")
                .bind(donation_id).execute(&mut *tx).await?;
        }
    }
    
    tx.commit().await?;
    Ok(row)
}
