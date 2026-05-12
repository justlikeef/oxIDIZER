use std::sync::{Arc, Mutex};
use async_trait::async_trait;
use futures::future::BoxFuture;
use secrecy::ExposeSecret;

use ox_security_core::{
    AuthResult, AuthPipelineContext, AuthSource, Credentials,
    drivers::AuthDriver,
};

use crate::drivers::ldap::{LdapAdapter, LdapAuthDriver, LdapBindResult, LdapConfig};

#[derive(Clone)]
pub struct AdConfig {
    pub ldap: LdapConfig,
    pub domain: String,
    pub upn_suffix: String,
}

#[derive(Clone)]
pub struct BindDnCapture {
    sequence: Arc<Mutex<Vec<LdapBindResult>>>,
    log: Arc<Mutex<Vec<String>>>,
}

impl BindDnCapture {
    pub fn new(result: LdapBindResult) -> Self {
        Self {
            sequence: Arc::new(Mutex::new(vec![result])),
            log: Arc::new(Mutex::new(vec![])),
        }
    }

    pub fn new_sequence(results: Vec<LdapBindResult>) -> Self {
        assert!(!results.is_empty(), "BindDnCapture::new_sequence requires ≥1 result");
        Self {
            sequence: Arc::new(Mutex::new(results)),
            log: Arc::new(Mutex::new(vec![])),
        }
    }

    pub fn last_bind_dn(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

impl LdapAdapter for BindDnCapture {
    fn bind_and_search(
        &self,
        _url: String,
        bind_dn: String,
        _password: String,
        _base_dn: String,
        _group_attr: String,
    ) -> BoxFuture<'static, LdapBindResult> {
        self.log.lock().unwrap().push(bind_dn);
        let result = {
            let mut seq = self.sequence.lock().unwrap();
            if seq.len() > 1 { seq.remove(0) } else { seq[0].clone() }
        };
        Box::pin(async move { result })
    }
}

pub struct AdAuthDriver {
    config: AdConfig,
    inner: LdapAuthDriver,
}

impl AdAuthDriver {
    pub fn new(config: AdConfig) -> Self {
        let inner = LdapAuthDriver::new(config.ldap.clone());
        Self { config, inner }
    }

    pub fn with_mock(config: AdConfig, mock: impl LdapAdapter) -> Self {
        let inner = LdapAuthDriver::with_mock(config.ldap.clone(), mock);
        Self { config, inner }
    }
}

#[async_trait]
impl AuthDriver for AdAuthDriver {
    async fn authenticate(
        &self,
        credentials: &Credentials,
        _ctx: &mut AuthPipelineContext,
    ) -> AuthResult {
        let (username, password) = match credentials {
            Credentials::UsernamePassword { username, password } => {
                (username.as_str(), password.expose_secret())
            }
            _ => return AuthResult::Continue,
        };

        let plain_dn = self.config.ldap.bind_dn_template.replace("{}", username);
        let netbios_dn = format!("{}\\{}", self.config.domain, username);
        let upn_dn = format!("{}@{}", username, self.config.upn_suffix);

        let candidates: &[(&str, &str)] = &[
            (&plain_dn, username),
            (&netbios_dn, username),
            (&upn_dn, username),
        ];

        let mut last_reject: Option<AuthResult> = None;
        for (bind_dn, display) in candidates {
            let result = self.inner.try_bind(bind_dn, password, display, AuthSource::Ad).await;
            match result {
                AuthResult::Authenticated(_) => return result,
                AuthResult::Reject(_) => { last_reject = Some(result); }
                other => return other,
            }
        }

        last_reject.unwrap_or_else(|| {
            AuthResult::Reject(format!("AD: all bind forms rejected for '{}'", username))
        })
    }
}
