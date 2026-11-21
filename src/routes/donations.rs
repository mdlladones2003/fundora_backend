use crate::{
    app::AppState,
    auth::{AuthUser, has_role},
    database::{
        models::donation::{CreateDonation, Donation, DonationReceipt},
        repositories::donation_repository as repo,
    },
    error::AppError,
};
use axum::{
    Json,
    extract::{Extension, Path, State},
};
use uuid::Uuid;

pub async fn create(
    State(s): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Json(x): Json<CreateDonation>,
) -> Result<Json<Donation>, AppError> {
    if !has_role(&user, &["donor"]) {
        return Err(AppError::Forbidden);
    }
    if x.amount <= rust_decimal::Decimal::ZERO
        || x.amount.fract() != rust_decimal::Decimal::ZERO
        || x.message
            .as_deref()
            .is_some_and(|message| message.len() > 500)
    {
        return Err(AppError::BadRequest(
            "Amount must be a positive whole number and messages must be 500 characters or fewer"
                .into(),
        ));
    }

    Ok(Json(repo::create(&s.pool, Some(user.id), x).await?))
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
        SELECT d.*, c.title AS campaign_title, t.tx_hash AS transaction_hash
             , t.network AS transaction_network, t.status AS transaction_status,
               t.verified AS transaction_verified
        FROM donations d
        JOIN campaigns c ON c.id=d.campaign_id
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
        SELECT d.*, c.title AS campaign_title, t.tx_hash AS transaction_hash,
               t.network AS transaction_network, t.status AS transaction_status,
               t.verified AS transaction_verified, t.block_number, t.confirmations,
               t.confirmed_at AS transaction_confirmed_at
        FROM donations d
        JOIN campaigns c ON c.id = d.campaign_id
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
