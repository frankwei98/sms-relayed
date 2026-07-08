#[cfg(test)]
mod tests {
    use super::auth::SessionStore;
    use super::config::{check_config_payload, CheckConfigPayload};
    use crate::config::AppConfig;

    #[test]
    fn login_cookie_uses_p2_session_contract() {
        let sessions = SessionStore::default();
        let cookie = sessions.login_cookie(false);

        assert!(cookie.starts_with("sms-relayed-session="));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Lax"));
        assert!(cookie.contains("Path=/"));
        assert!(cookie.contains("Max-Age=604800"));
        assert!(!cookie.contains("Secure"));
    }

    #[test]
    fn login_cookie_is_secure_when_request_is_https() {
        let sessions = SessionStore::default();
        let cookie = sessions.login_cookie(true);
        assert!(cookie.contains("Secure"));
    }

    #[test]
    fn session_tokens_expire_after_seven_days() {
        let sessions = SessionStore::default();
        let token = sessions.create_session();
        assert!(sessions.is_valid(&token));

        sessions.expire_for_test(&token);
        assert!(!sessions.is_valid(&token));
    }

    #[test]
    fn config_check_accepts_json_or_toml_and_rejects_bad_config() {
        let mut cfg = AppConfig::default();
        cfg.api.password = "secret".to_string();

        assert!(check_config_payload(CheckConfigPayload::Json(cfg.clone())).is_ok());

        let toml = toml::to_string_pretty(&cfg).unwrap();
        assert!(check_config_payload(CheckConfigPayload::Toml { toml }).is_ok());

        cfg.api.password.clear();
        let err = check_config_payload(CheckConfigPayload::Json(cfg))
            .unwrap_err()
            .to_string();
        assert!(err.contains("api.password"));
    }
}
