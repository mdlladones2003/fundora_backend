use axum::{
    extract::{Path,State,Extension},
    Json
};
use uuid::Uuid;
use crate::{
    app::AppState,
    auth::{AuthUser,has_role},
    database::models::withdrawal::{CreateWithdrawal,ReviewWithdrawal,Withdrawal},
    error::AppError
};

pub async fn create(State(s):State<AppState>, Extension(user):Extension<AuthUser>, Json(x):Json<CreateWithdrawal>) -> Result<Json<Withdrawal>,AppError> {
    if !has_role(&user, &["organizer", "administrator"]) {
        return Err(AppError::Forbidden);
    }
    
    if x.amount <= rust_decimal::Decimal::ZERO || x.destination_address.trim().is_empty() {
        return Err(AppError::BadRequest("positive amount and destination are required".into()));
    }
    
    let row = sqlx::query_as::<_,Withdrawal>(r#"
            INSERT INTO withdrawals(campaign_id,requester_id,amount,destination_address,purpose)
            SELECT $1,$2,$3,$4,$5 WHERE EXISTS(SELECT 1 FROM campaigns WHERE id=$1 AND organizer_id=$2)
            RETURNING *
        "#)
        .bind(x.campaign_id)
        .bind(user.id)
        .bind(x.amount)
        .bind(x.destination_address)
        .bind(x.purpose)
        .fetch_one(&s.pool).await?;
    
    Ok(Json(row))
}

pub async fn review(State(s):State<AppState>, Extension(user):Extension<AuthUser>, Path(id):Path<Uuid>, Json(x):Json<ReviewWithdrawal>) -> Result<Json<Withdrawal>,AppError> {
    if !has_role(&user, &["administrator"]) {
        return Err(AppError::Forbidden);
    }
    
    let status = if x.approve {"approved"} else {"rejected"};
    let row = sqlx::query_as::<_,Withdrawal>("UPDATE withdrawals SET status=$2,reviewed_by=$3,reviewed_at=now() WHERE id=$1 AND status='pending' RETURNING *")
                                .bind(id)
                                .bind(status)
                                .bind(user.id)
                                .fetch_optional(&s.pool).await?.ok_or(AppError::NotFound)?;
    
    Ok(Json(row))
}
