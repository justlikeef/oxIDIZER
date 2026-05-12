//! TOTP MFA authentication driver.
//!
//! This driver handles the second factor in a two-step pipeline.
//! It accepts only `Credentials::MfaPasscode` and validates the 6-digit code
//! against the user's TOTP secret using RFC 6238 (SHA-1, 30-second steps, ±1 window).
//!
//! For all other credential variants, and when no `partial_principal` is set in
//! the pipeline context, it returns `AuthResult::Continue` so the pipeline can
//! pass through to the next driver.

use std::sync::Arc;
use async_trait::async_trait;
use totp_rs::{Algorithm, TOTP, Secret};
use ox_security_core::{
    AuthResult, AuthPipelineContext, Credentials, PrincipalId,
    drivers::AuthDriver,
};

/// Returns the base32-encoded TOTP secret for a given `PrincipalId`, or `None`
/// if the user has no TOTP configured.
///
/// In production this will query the user store (via a data crate); in tests it
/// is an in-closure lookup over a fixed map.
pub type TotpSecretLookupFn = Arc<dyn Fn(&PrincipalId) -> Option<String> + Send + Sync>;

/// TOTP MFA authentication driver.
///
/// Position in pipeline: after the primary credential driver (e.g. `DbAuthDriver`)
/// which sets `ctx.partial_principal` and returns `MfaRequired`.
pub struct TotpAuthDriver {
    lookup: TotpSecretLookupFn,
}

impl TotpAuthDriver {
    pub fn new(lookup: TotpSecretLookupFn) -> Self {
        Self { lookup }
    }
}

/// Validate a 6-digit TOTP code against a base32-encoded secret.
/// Accepts codes from the current 30-second step and the immediately
/// preceding and following steps (±1 window) to tolerate clock skew.
///
/// Returns `true` if the code is valid for any of the three steps.
fn validate_totp(secret_b32: &str, code: &str) -> bool {
    let secret_bytes = match Secret::Encoded(secret_b32.to_string()).to_bytes() {
        Ok(b) => b,
        Err(_) => return false,
    };

    // Use new_unchecked to allow secrets of any size, including well-known
    // test vectors shorter than the RFC-recommended 128 bits.
    let totp = TOTP::new_unchecked(
        Algorithm::SHA1,
        6,
        1,
        30,
        secret_bytes,
    );

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Check current step and ±1 for clock skew tolerance.
    for offset in [-1i64, 0, 1] {
        let t = (now_secs as i64 + offset * 30) as u64;
        if totp.generate(t) == code {
            return true;
        }
    }
    false
}

#[async_trait]
impl AuthDriver for TotpAuthDriver {
    async fn authenticate(
        &self,
        credentials: &Credentials,
        ctx: &mut AuthPipelineContext,
    ) -> AuthResult {
        // Only handle MfaPasscode credentials.
        let code = match credentials {
            Credentials::MfaPasscode { code, .. } => code.as_str(),
            _ => return AuthResult::Continue,
        };

        // Only proceed if a partial principal has been set by a prior driver.
        let partial = match ctx.partial_principal.take() {
            Some(p) => p,
            None => return AuthResult::Continue,
        };

        // Look up the TOTP secret for this principal.
        let secret = match (self.lookup)(&partial.id) {
            Some(s) => s,
            None => {
                return AuthResult::Reject(format!(
                    "no TOTP secret configured for principal '{}'",
                    partial.id.as_uuid()
                ));
            }
        };

        if validate_totp(&secret, code) {
            AuthResult::Authenticated(partial.into_principal(None))
        } else {
            AuthResult::Reject("invalid TOTP code".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    use std::str::FromStr;
    use ox_security_core::{
        AuthPipelineContext, AuthResult, Credentials,
        PrincipalId, PartialPrincipal, AuthSource, TenantId, SessionToken,
        GroupId,
    };
    use totp_rs::{Algorithm, TOTP, Secret};

    fn make_partial_principal(id: PrincipalId) -> PartialPrincipal {
        PartialPrincipal {
            id,
            display_name: "alice".to_string(),
            source: AuthSource::Local,
            groups: vec![GroupId::new("users")],
            tenant_id: TenantId::from_str("test").unwrap(),
        }
    }

    fn make_ctx_with_partial(pp: PartialPrincipal) -> AuthPipelineContext {
        AuthPipelineContext {
            partial_principal: Some(pp),
            tenant_id: TenantId::from_str("test").unwrap(),
            source_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        }
    }

    fn make_ctx_empty() -> AuthPipelineContext {
        AuthPipelineContext {
            partial_principal: None,
            tenant_id: TenantId::from_str("test").unwrap(),
            source_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        }
    }

    /// Generate a valid TOTP code for a given base32 secret at the current time.
    fn current_code(secret_b32: &str) -> String {
        let totp = TOTP::new_unchecked(
            Algorithm::SHA1,
            6,
            1,
            30,
            Secret::Encoded(secret_b32.to_string()).to_bytes().unwrap(),
        );
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        totp.generate(t)
    }

    // ── Test 1 ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn totp_continues_for_non_mfa_creds() {
        use secrecy::SecretString;
        let driver = TotpAuthDriver::new(Arc::new(|_id| Some("JBSWY3DPEHPK3PXP".to_string())));
        let creds = Credentials::UsernamePassword {
            username: "alice".to_string(),
            password: SecretString::new("pass".to_string()),
        };
        let mut ctx = make_ctx_empty();
        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(matches!(result, AuthResult::Continue));
    }

    // ── Test 2 ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn totp_continues_when_no_partial_principal() {
        let driver = TotpAuthDriver::new(Arc::new(|_id| Some("JBSWY3DPEHPK3PXP".to_string())));
        let creds = Credentials::MfaPasscode {
            session_token: SessionToken::new(),
            code: "123456".to_string(),
        };
        let mut ctx = make_ctx_empty(); // no partial principal
        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(matches!(result, AuthResult::Continue));
    }

    // ── Test 3 ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn totp_validates_correct_code() {
        let secret_b32 = "JBSWY3DPEHPK3PXP";
        let id = PrincipalId::new();
        let id_clone = id.clone();

        let driver = TotpAuthDriver::new(Arc::new(move |pid: &PrincipalId| {
            if pid == &id_clone {
                Some(secret_b32.to_string())
            } else {
                None
            }
        }));

        let code = current_code(secret_b32);
        let creds = Credentials::MfaPasscode {
            session_token: SessionToken::new(),
            code,
        };
        let pp = make_partial_principal(id);
        let mut ctx = make_ctx_with_partial(pp);

        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(
            matches!(result, AuthResult::Authenticated(_)),
            "expected Authenticated, got something else"
        );
        if let AuthResult::Authenticated(p) = result {
            assert_eq!(p.display_name, "alice");
            assert!(matches!(p.source, AuthSource::Local));
        }
    }

    // ── Test 4 ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn totp_rejects_wrong_code() {
        let secret_b32 = "JBSWY3DPEHPK3PXP";
        let id = PrincipalId::new();
        let id_clone = id.clone();

        let driver = TotpAuthDriver::new(Arc::new(move |pid: &PrincipalId| {
            if pid == &id_clone {
                Some(secret_b32.to_string())
            } else {
                None
            }
        }));

        let creds = Credentials::MfaPasscode {
            session_token: SessionToken::new(),
            code: "000000".to_string(), // deliberately wrong
        };
        let pp = make_partial_principal(id);
        let mut ctx = make_ctx_with_partial(pp);

        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(matches!(result, AuthResult::Reject(_)));
    }

    // ── Test 5 ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn totp_rejects_when_no_secret_configured() {
        // Lookup returns None — user has no TOTP secret enrolled.
        // Even with a valid-looking code, must Reject.
        let driver = TotpAuthDriver::new(Arc::new(|_id: &PrincipalId| None));

        let creds = Credentials::MfaPasscode {
            session_token: SessionToken::new(),
            code: "123456".to_string(),
        };
        let pp = make_partial_principal(PrincipalId::new());
        let mut ctx = make_ctx_with_partial(pp);

        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(
            matches!(result, AuthResult::Reject(_)),
            "must Reject when user has no TOTP secret configured"
        );
    }
}
