use serde_json::Value;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub async fn record_audit(
    tx: &mut Transaction<'_, Postgres>,
    actor_id: Option<Uuid>,
    action: &str,
    entity_type: &str,
    entity_id: Option<Uuid>,
    details: Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO audit_logs(actor_id, action, entity_type, entity_id, details)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(actor_id)
    .bind(action)
    .bind(entity_type)
    .bind(entity_id)
    .bind(details)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn notify_user(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    title: &str,
    body: &str,
    href: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO notifications(user_id, title, body, href) VALUES ($1, $2, $3, $4)")
        .bind(user_id)
        .bind(title)
        .bind(body)
        .bind(href)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub async fn notify_role(
    tx: &mut Transaction<'_, Postgres>,
    role: &str,
    title: &str,
    body: &str,
    href: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO notifications(user_id, title, body, href)
        SELECT ur.user_id, $2, $3, $4
        FROM user_roles ur
        JOIN roles r ON r.id = ur.role_id
        JOIN users u ON u.id = ur.user_id
        WHERE r.name = $1 AND u.is_active = TRUE
        "#,
    )
    .bind(role)
    .bind(title)
    .bind(body)
    .bind(href)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
