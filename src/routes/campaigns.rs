use axum::{
    extract::{ Path, State, Extension}, Json
};
use uuid::Uuid;
use crate::{
    app::AppState, 
    auth::{AuthUser, has_role},
    database::{
        models::campaign::{ CreateCampaign, UpdateCampaign },
        repositories::campaign_repository as repo
    },
    error::AppError
};

pub async fn list(State(s): State<AppState>) -> Result<Json<Vec<crate::database::models::campaign::Campaign>>, AppError> {
    Ok(Json(repo::list(&s.pool).await?))
}

pub async fn get(State(s): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<crate::database::models::campaign::Campaign>, AppError> {
    repo::get(&s.pool,id).await?.map(Json).ok_or(AppError::NotFound)
}

pub async fn create(State(s): State<AppState>, Extension(user): Extension<AuthUser>, Json(x): Json<CreateCampaign>) -> Result<Json<crate::database::models::campaign::Campaign>, AppError> {
    if !has_role(&user, &["organizer", "administrator"]) {
        return Err(AppError::Forbidden);
    }

    if x.title.trim().is_empty() || x.goal_amount <= rust_decimal::Decimal::ZERO {
        return Err(AppError::BadRequest("title and positive goal_amount are required".into()));
    }
    Ok(Json(repo::create(&s.pool,user.id,x).await?))
}

pub async fn update(State(s): State<AppState>, Extension(user): Extension<AuthUser>, Path(id): Path<Uuid>, Json(x): Json<UpdateCampaign>) -> Result<Json<crate::database::models::campaign::Campaign>, AppError> {
    if !has_role(&user, &["organizer", "administrator"]) {
        return Err(AppError::Forbidden);
    }
    repo::update(&s.pool,id,user.id,x).await?.map(Json).ok_or(AppError::NotFound)
}
