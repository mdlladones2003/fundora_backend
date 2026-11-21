#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub port: u16,
    pub jwt_secret: String,
    pub jwt_issuer: String,
    pub blockchain_webhook_secret: String,
    pub frontend_origin: String,
    pub email_from: String,
    pub resend_api_key: String,
    pub google_client_id: String,
    pub apple_client_id: String,
}

impl Config {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let jwt_secret = std::env::var("JWT_SECRET")?;

        if jwt_secret.len() < 32 {
            return Err("JWT_SECRET must be at least 32 characters".into());
        }

        Ok(Self {
            database_url: std::env::var("DATABASE_URL")?,
            port: std::env::var("PORT")
                .unwrap_or_else(|_| "8080".into())
                .parse()?,
            jwt_secret,
            jwt_issuer: std::env::var("JWT_ISSUER").unwrap_or_else(|_| "fundora".into()),
            blockchain_webhook_secret: std::env::var("BLOCKCHAIN_WEBHOOK_SECRET")
                .unwrap_or_default(),
            frontend_origin: std::env::var("FRONTEND_ORIGIN")
                .unwrap_or_else(|_| "http://localhost:3000".into()),
            email_from: std::env::var("EMAIL_FROM").unwrap_or_default(),
            resend_api_key: std::env::var("RESEND_API_KEY").unwrap_or_default(),
            google_client_id: std::env::var("GOOGLE_CLIENT_ID").unwrap_or_default(),
            apple_client_id: std::env::var("APPLE_CLIENT_ID").unwrap_or_default(),
        })
    }
}
