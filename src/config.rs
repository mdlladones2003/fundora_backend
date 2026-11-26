#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PaymentMode {
    Disabled,
    Mock,
    Xendit,
}

#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub port: u16,
    pub jwt_secret: String,
    pub jwt_issuer: String,
    pub frontend_origin: String,
    pub xendit_return_origin: Option<String>,
    pub email_from: String,
    pub resend_api_key: String,
    pub google_client_id: String,
    pub apple_client_id: String,
    pub siwe_domain: String,
    pub siwe_uri: String,
    pub siwe_chain_id: Option<u64>,
    pub evm_rpc_url: Option<String>,
    pub evm_chain_id: u64,
    pub evm_donation_contract: Option<String>,
    pub evm_min_confirmations: u64,
    pub payment_mode: PaymentMode,
    pub xendit_secret_key: Option<String>,
    pub xendit_webhook_token: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let jwt_secret = std::env::var("JWT_SECRET")?;
        let siwe_chain_id = std::env::var("SIWE_CHAIN_ID")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(|value| value.parse::<u64>())
            .transpose()?;
        let evm_min_confirmations = std::env::var("EVM_MIN_CONFIRMATIONS")
            .unwrap_or_else(|_| "2".into())
            .parse::<u64>()?;
        let payment_mode = match std::env::var("PAYMENT_MODE")
            .unwrap_or_else(|_| {
                if cfg!(debug_assertions) {
                    "mock".into()
                } else {
                    "disabled".into()
                }
            })
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "disabled" => PaymentMode::Disabled,
            "mock" if cfg!(debug_assertions) => PaymentMode::Mock,
            "mock" => return Err("PAYMENT_MODE=mock is only available in debug builds".into()),
            "xendit" => PaymentMode::Xendit,
            _ => return Err("PAYMENT_MODE must be disabled, mock, or xendit".into()),
        };
        let xendit_secret_key = std::env::var("XENDIT_SECRET_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let xendit_webhook_token = std::env::var("XENDIT_WEBHOOK_TOKEN")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let xendit_return_origin = std::env::var("XENDIT_RETURN_ORIGIN")
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());

        if jwt_secret.len() < 32 || evm_min_confirmations == 0 {
            return Err("JWT_SECRET must be at least 32 characters and EVM_MIN_CONFIRMATIONS must be positive".into());
        }
        if payment_mode == PaymentMode::Xendit
            && (xendit_secret_key.is_none() || xendit_webhook_token.is_none())
        {
            return Err(
                "XENDIT_SECRET_KEY and XENDIT_WEBHOOK_TOKEN are required when PAYMENT_MODE=xendit"
                    .into(),
            );
        }
        if payment_mode == PaymentMode::Xendit
            && !xendit_secret_key
                .as_deref()
                .is_some_and(|key| key.starts_with("xnd_development_"))
        {
            return Err(
                "Only an Xendit Test Mode secret key is accepted when PAYMENT_MODE=xendit".into(),
            );
        }
        if xendit_return_origin
            .as_deref()
            .is_some_and(|origin| !valid_xendit_return_origin(origin))
        {
            return Err("XENDIT_RETURN_ORIGIN must be a valid HTTPS URL".into());
        }

        Ok(Self {
            database_url: std::env::var("DATABASE_URL")?,
            port: std::env::var("PORT")
                .unwrap_or_else(|_| "8080".into())
                .parse()?,
            jwt_secret,
            jwt_issuer: std::env::var("JWT_ISSUER").unwrap_or_else(|_| "fundora".into()),
            frontend_origin: std::env::var("FRONTEND_ORIGIN")
                .unwrap_or_else(|_| "http://localhost:5173".into()),
            xendit_return_origin,
            email_from: std::env::var("EMAIL_FROM").unwrap_or_default(),
            resend_api_key: std::env::var("RESEND_API_KEY").unwrap_or_default(),
            google_client_id: std::env::var("GOOGLE_CLIENT_ID").unwrap_or_default(),
            apple_client_id: std::env::var("APPLE_CLIENT_ID").unwrap_or_default(),
            siwe_domain: std::env::var("SIWE_DOMAIN").unwrap_or_default(),
            siwe_uri: std::env::var("SIWE_URI").unwrap_or_default(),
            siwe_chain_id,
            evm_rpc_url: std::env::var("EVM_RPC_URL")
                .ok()
                .filter(|value| !value.trim().is_empty()),
            evm_chain_id: std::env::var("EVM_CHAIN_ID")
                .unwrap_or_else(|_| "11155111".into())
                .parse()?,
            evm_donation_contract: std::env::var("EVM_DONATION_CONTRACT")
                .ok()
                .filter(|value| !value.trim().is_empty()),
            evm_min_confirmations,
            payment_mode,
            xendit_secret_key,
            xendit_webhook_token,
        })
    }
}

fn valid_xendit_return_origin(value: &str) -> bool {
    reqwest::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
    })
}
