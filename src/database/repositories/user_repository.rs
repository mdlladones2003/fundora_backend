use sqlx::PgPool;
use uuid::Uuid;
use crate::database::models::user::User;

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as::<_, User>("SELECT * FROM users WHERE id=$1 AND is_active=TRUE")
        .bind(id)
        .fetch_optional(pool)
        .await
}
