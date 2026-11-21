use axum::{extract::{Path, State, Extension}, Json};
use uuid::Uuid;
use crate::{
    app::AppState,
    auth::{AuthUser, has_role},
    database::{
        models::donation::{CreateDonation, Donation},
        repositories::donation_repository as repo
    },
    error::AppError
};

pub async fn create(State(s): State<AppState>, Extension(user): Extension<AuthUser>, Json(x): Json<CreateDonation>) -> Result<Json<Donation>, AppError> {
    if !has_role(&user, &["donor", "administrator"]) {
        return Err(AppError::Forbidden);
    }
    if x.amount <= rust_decimal::Decimal::ZERO {
        return Err(AppError::BadRequest("amount must be positive".into()));
    }
    
    Ok(Json(repo::create(&s.pool,Some(user.id), x).await?))
}

pub async fn list_for_campaign(State(s): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Vec<Donation>>, AppError> {
    Ok(Json(repo::list_for_campaign(&s.pool, id).await?))
}
