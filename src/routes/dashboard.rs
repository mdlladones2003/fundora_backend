use axum::{
    Json,
    extract::{Extension, State},
};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    app::AppState,
    auth::{AuthUser, has_role},
    error::AppError,
};

#[derive(Debug, Serialize, FromRow)]
pub struct DailyStatistic {
    pub date: String,
    pub currency: String,
    pub amount: Decimal,
}

#[derive(Debug, Serialize, FromRow)]
pub struct TopDonor {
    pub name: String,
    pub currency: String,
    pub amount: Decimal,
}

#[derive(Debug, Serialize, FromRow)]
pub struct CurrencyTotal {
    pub currency: String,
    pub amount: Decimal,
}

#[derive(Debug, Serialize)]
pub struct DashboardSummary {
    pub total_donations: Vec<CurrencyTotal>,
    pub donations_today: Vec<CurrencyTotal>,
    pub daily_statistics: Vec<DailyStatistic>,
    pub top_donors: Vec<TopDonor>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct OrganizerDonation {
    pub id: Uuid,
    pub campaign_id: Uuid,
    pub campaign_title: String,
    pub donor_name: String,
    pub amount: Decimal,
    pub currency: String,
    pub payment_amount: Option<Decimal>,
    pub payment_currency: Option<String>,
    pub exchange_rate: Option<Decimal>,
    pub exchange_rate_provider: Option<String>,
    pub exchange_rate_at: Option<DateTime<Utc>>,
    pub status: String,
    pub payment_method: Option<String>,
    pub payment_provider: Option<String>,
    pub payment_reference: Option<String>,
    pub created_at: DateTime<Utc>,
    pub confirmed_at: Option<DateTime<Utc>>,
    pub transaction_hash: Option<String>,
    pub transaction_network: Option<String>,
    pub transaction_status: Option<String>,
    pub transaction_verified: Option<bool>,
}

fn dashboard_role(user: &AuthUser) -> Result<&'static str, AppError> {
    match user.role.as_str() {
        "donor" => Ok("donor"),
        "organizer" => Ok("organizer"),
        "administrator" => Ok("administrator"),
        _ => Err(AppError::Forbidden),
    }
}

pub async fn summary(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<DashboardSummary>, AppError> {
    let role = dashboard_role(&user)?;

    let total_donations = sqlx::query_as::<_, CurrencyTotal>(
        r#"
        SELECT d.currency, SUM(d.amount) AS amount
        FROM donations d
        LEFT JOIN campaigns c ON c.id = d.campaign_id
        WHERE d.status = 'confirmed'
          AND (
            ($1 = 'donor' AND d.donor_id = $2)
            OR ($1 = 'organizer' AND c.organizer_id = $2)
            OR $1 = 'administrator'
          )
        GROUP BY d.currency
        ORDER BY d.currency
        "#,
    )
    .bind(role)
    .bind(user.id)
    .fetch_all(&state.pool)
    .await?;

    let donations_today = sqlx::query_as::<_, CurrencyTotal>(
        r#"
        SELECT d.currency, SUM(d.amount) AS amount
        FROM donations d
        LEFT JOIN campaigns c ON c.id = d.campaign_id
        WHERE d.status = 'confirmed'
          AND d.created_at >= CURRENT_DATE
          AND d.created_at < CURRENT_DATE + INTERVAL '1 day'
          AND (
            ($1 = 'donor' AND d.donor_id = $2)
            OR ($1 = 'organizer' AND c.organizer_id = $2)
            OR $1 = 'administrator'
          )
        GROUP BY d.currency
        ORDER BY d.currency
        "#,
    )
    .bind(role)
    .bind(user.id)
    .fetch_all(&state.pool)
    .await?;

    let daily_statistics = sqlx::query_as::<_, DailyStatistic>(
        r#"
        WITH days AS (
            SELECT GENERATE_SERIES(
                CURRENT_DATE - INTERVAL '6 days',
                CURRENT_DATE,
                INTERVAL '1 day'
            ) AS day
        ), currencies AS (
            SELECT DISTINCT d.currency
            FROM donations d
            LEFT JOIN campaigns c ON c.id = d.campaign_id
            WHERE d.status = 'confirmed'
              AND d.created_at >= CURRENT_DATE - INTERVAL '6 days'
              AND (
                ($1 = 'donor' AND d.donor_id = $2)
                OR ($1 = 'organizer' AND c.organizer_id = $2)
                OR $1 = 'administrator'
              )
        )
        SELECT TO_CHAR(days.day, 'YYYY-MM-DD') AS date,
               currencies.currency,
               COALESCE(SUM(d.amount), 0) AS amount
        FROM days CROSS JOIN currencies
        LEFT JOIN donations d
          ON d.status = 'confirmed'
         AND d.currency = currencies.currency
         AND d.created_at >= days.day
         AND d.created_at < days.day + INTERVAL '1 day'
         AND (
            ($1 = 'donor' AND d.donor_id = $2)
            OR (
                $1 = 'organizer'
                AND EXISTS (
                    SELECT 1 FROM campaigns owned
                    WHERE owned.id = d.campaign_id AND owned.organizer_id = $2
                )
            )
            OR $1 = 'administrator'
         )
        GROUP BY days.day, currencies.currency
        ORDER BY currencies.currency, days.day
        "#,
    )
    .bind(role)
    .bind(user.id)
    .fetch_all(&state.pool)
    .await?;

    let top_donors = sqlx::query_as::<_, TopDonor>(
        r#"
        WITH ranked_donors AS (
        SELECT COALESCE(u.display_name, 'Fundora donor') AS name,
               d.currency,
               SUM(d.amount) AS amount,
               ROW_NUMBER() OVER (PARTITION BY d.currency ORDER BY SUM(d.amount) DESC) AS rank
        FROM donations d
        JOIN users u ON u.id = d.donor_id
        LEFT JOIN campaigns c ON c.id = d.campaign_id
        WHERE d.status = 'confirmed'
          AND d.is_anonymous = FALSE
          AND (
            ($1 = 'organizer' AND c.organizer_id = $2)
            OR $1 IN ('donor', 'administrator')
          )
        GROUP BY u.id, u.display_name, d.currency
        )
        SELECT name, currency, amount FROM ranked_donors
        WHERE rank <= 5
        ORDER BY currency, amount DESC
        "#,
    )
    .bind(role)
    .bind(user.id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(DashboardSummary {
        total_donations,
        donations_today,
        daily_statistics,
        top_donors,
    }))
}

pub async fn organizer_donations(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<Vec<OrganizerDonation>>, AppError> {
    if !has_role(&user, &["organizer", "administrator"]) {
        return Err(AppError::Forbidden);
    }

    let donations = sqlx::query_as::<_, OrganizerDonation>(
        r#"
        SELECT d.id, d.campaign_id, c.title AS campaign_title,
               CASE WHEN d.is_anonymous THEN 'Anonymous donor'
                    ELSE COALESCE(u.display_name, 'Fundora donor') END AS donor_name,
               d.amount, d.currency, d.payment_amount, d.payment_currency,
               d.exchange_rate, d.exchange_rate_provider, d.exchange_rate_at,
               d.status, p.method AS payment_method, p.provider AS payment_provider,
               p.session_id AS payment_reference, d.created_at, d.confirmed_at,
               t.tx_hash AS transaction_hash, t.network AS transaction_network,
               t.status AS transaction_status, t.verified AS transaction_verified
        FROM donations d
        JOIN campaigns c ON c.id = d.campaign_id
        LEFT JOIN users u ON u.id = d.donor_id
        LEFT JOIN payment_attempts p ON p.donation_id = d.id
        LEFT JOIN LATERAL (
            SELECT tx_hash, network, status, verified FROM blockchain_transactions
            WHERE donation_id = d.id
            ORDER BY created_at DESC
            LIMIT 1
        ) t ON TRUE
        WHERE c.organizer_id = $1
        ORDER BY d.created_at DESC
        "#,
    )
    .bind(user.id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(donations))
}
