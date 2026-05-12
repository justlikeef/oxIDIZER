use std::sync::Arc;
use async_trait::async_trait;
use futures::future::BoxFuture;
use secrecy::ExposeSecret;

use ox_security_core::{
    AuthResult, AuthPipelineContext, AuthSource, Credentials,
    GroupId, Principal, PrincipalId, TenantId,
    drivers::AuthDriver,
};

#[derive(Clone)]
pub struct LdapConfig {
    pub url: String,
    pub bind_dn_template: String,
    pub base_dn: String,
    pub group_attr: String,
    pub tenant_id: TenantId,
}

#[derive(Clone, Debug)]
pub enum LdapBindResult {
    Success { groups: Vec<String> },
    InvalidCredentials,
    NoSuchEntry,
    Error(String),
}

pub trait LdapAdapter: Send + Sync + 'static {
    fn bind_and_search(
        &self,
        url: String,
        bind_dn: String,
        password: String,
        base_dn: String,
        group_attr: String,
    ) -> BoxFuture<'static, LdapBindResult>;
}

fn escape_ldap_filter_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\5c"),
            '*'  => out.push_str("\\2a"),
            '('  => out.push_str("\\28"),
            ')'  => out.push_str("\\29"),
            '\0' => out.push_str("\\00"),
            c    => out.push(c),
        }
    }
    out
}

pub(crate) struct RealLdapAdapter;

impl LdapAdapter for RealLdapAdapter {
    fn bind_and_search(
        &self,
        url: String,
        bind_dn: String,
        password: String,
        base_dn: String,
        group_attr: String,
    ) -> BoxFuture<'static, LdapBindResult> {
        Box::pin(async move {
            use ldap3::{LdapConnAsync, Scope, SearchEntry};

            let (conn, mut ldap) = match LdapConnAsync::new(&url).await {
                Ok(pair) => pair,
                Err(e) => return LdapBindResult::Error(e.to_string()),
            };
            ldap3::drive!(conn);

            match ldap.simple_bind(&bind_dn, &password).await {
                Err(e) => return LdapBindResult::Error(e.to_string()),
                Ok(res) => match res.rc {
                    0 => {}
                    49 => return LdapBindResult::InvalidCredentials,
                    32 => return LdapBindResult::NoSuchEntry,
                    code => return LdapBindResult::Error(format!("LDAP bind returned rc={}", code)),
                },
            }

            let username_part = bind_dn
                .split(',')
                .next()
                .and_then(|rdn| rdn.split('=').nth(1))
                .unwrap_or("*");
            let filter = format!("(uid={})", escape_ldap_filter_value(username_part));
            let attr_ref = group_attr.as_str();

            let groups = match ldap.search(&base_dn, Scope::Subtree, &filter, vec![attr_ref]).await {
                Err(e) => return LdapBindResult::Error(e.to_string()),
                Ok(res) => match res.success() {
                    Err(e) => return LdapBindResult::Error(e.to_string()),
                    Ok((entries, _)) => {
                        let mut groups = Vec::new();
                        for entry in entries {
                            let e = SearchEntry::construct(entry);
                            if let Some(vals) = e.attrs.get(&group_attr) {
                                for v in vals {
                                    groups.push(v.clone());
                                }
                            }
                        }
                        groups
                    }
                },
            };

            let _ = ldap.unbind().await;
            LdapBindResult::Success { groups }
        })
    }
}

#[cfg(any(test, feature = "test-support"))]
pub struct MockLdapAdapter {
    result: LdapBindResult,
}

#[cfg(any(test, feature = "test-support"))]
impl MockLdapAdapter {
    pub fn new(result: LdapBindResult) -> Self {
        Self { result }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl LdapAdapter for MockLdapAdapter {
    fn bind_and_search(
        &self,
        _url: String,
        _bind_dn: String,
        _password: String,
        _base_dn: String,
        _group_attr: String,
    ) -> BoxFuture<'static, LdapBindResult> {
        let result = self.result.clone();
        Box::pin(async move { result })
    }
}

pub struct LdapAuthDriver {
    config: LdapConfig,
    adapter: Arc<dyn LdapAdapter>,
}

impl LdapAuthDriver {
    pub fn new(config: LdapConfig) -> Self {
        Self { config, adapter: Arc::new(RealLdapAdapter) }
    }

    pub fn with_mock(config: LdapConfig, mock: impl LdapAdapter) -> Self {
        Self { config, adapter: Arc::new(mock) }
    }

    pub(crate) async fn try_bind(
        &self,
        bind_dn: &str,
        password: &str,
        display_name: &str,
        auth_source: AuthSource,
    ) -> AuthResult {
        let result = self.adapter.bind_and_search(
            self.config.url.clone(),
            bind_dn.to_string(),
            password.to_string(),
            self.config.base_dn.clone(),
            self.config.group_attr.clone(),
        ).await;

        match result {
            LdapBindResult::Success { groups } => {
                let group_ids: Vec<GroupId> = groups.into_iter().map(GroupId::new).collect();
                AuthResult::Authenticated(Principal {
                    id: PrincipalId::new(),
                    display_name: display_name.to_string(),
                    source: auth_source,
                    groups: group_ids,
                    tenant_id: self.config.tenant_id.clone(),
                    session_id: None,
                })
            }
            LdapBindResult::InvalidCredentials => {
                AuthResult::Reject(format!("invalid credentials for '{}'", display_name))
            }
            LdapBindResult::NoSuchEntry => {
                AuthResult::Reject(format!("user '{}' not found in directory", display_name))
            }
            LdapBindResult::Error(_msg) => {
                // Infrastructure error (connection failure, timeout, TLS error).
                // Return Continue so the pipeline can try the next driver; the pipeline
                // is fail-closed at the system level — no driver authenticating = reject.
                AuthResult::Continue
            }
        }
    }
}

#[async_trait]
impl AuthDriver for LdapAuthDriver {
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

        let bind_dn = self.config.bind_dn_template.replace("{}", username);
        self.try_bind(&bind_dn, password, username, AuthSource::Ldap).await
    }
}
