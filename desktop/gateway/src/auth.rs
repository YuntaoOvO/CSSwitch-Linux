#[derive(Debug, PartialEq, Eq)]
pub enum AuthResult {
    Ok(String),
    Forbidden,
}

pub fn strip_path_secret(path: &str, secret: Option<&str>) -> AuthResult {
    let Some(secret) = secret.filter(|s| !s.is_empty()) else {
        return AuthResult::Ok(path.to_string());
    };
    let prefix = format!("/{secret}");
    if path == prefix {
        return AuthResult::Ok("/".to_string());
    }
    if let Some(rest) = path.strip_prefix(&(prefix + "/")) {
        return AuthResult::Ok(format!("/{rest}"));
    }
    AuthResult::Forbidden
}

#[cfg(test)]
mod tests {
    use super::{strip_path_secret, AuthResult};

    #[test]
    fn path_secret_strips_prefix_or_forbids() {
        assert_eq!(
            strip_path_secret("/v1/models", None),
            AuthResult::Ok("/v1/models".into())
        );
        assert_eq!(
            strip_path_secret("/s/v1/models", Some("s")),
            AuthResult::Ok("/v1/models".into())
        );
        assert_eq!(
            strip_path_secret("/s", Some("s")),
            AuthResult::Ok("/".into())
        );
        assert_eq!(
            strip_path_secret("/bad/v1/models", Some("s")),
            AuthResult::Forbidden
        );
    }
}
