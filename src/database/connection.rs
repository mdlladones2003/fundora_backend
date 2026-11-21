use sqlx::{PgPool, postgres::PgPoolOptions};

pub async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new().max_connections(20).connect(url).await
}
