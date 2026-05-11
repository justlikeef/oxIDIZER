use std::sync::Arc;
use ox_security_auth::AuthPipeline;
use ox_security_authz::AuthzPipeline;
use ox_security_accounting::AccountingPipeline;
use ox_security_core::{AuthDriver, AuthzDriver, AccountingDriver};
use crate::pipeline::SecurityPipeline;

pub struct SecurityPipelineBuilder {
    auth_drivers: Vec<Arc<dyn AuthDriver>>,
    authz_drivers: Vec<Arc<dyn AuthzDriver>>,
    accounting_drivers: Vec<Arc<dyn AccountingDriver>>,
}

impl SecurityPipelineBuilder {
    pub fn new() -> Self {
        Self {
            auth_drivers: Vec::new(),
            authz_drivers: Vec::new(),
            accounting_drivers: Vec::new(),
        }
    }

    pub fn auth(mut self, driver: Arc<dyn AuthDriver>) -> Self {
        self.auth_drivers.push(driver);
        self
    }

    pub fn authz(mut self, driver: Arc<dyn AuthzDriver>) -> Self {
        self.authz_drivers.push(driver);
        self
    }

    pub fn accounting(mut self, driver: Arc<dyn AccountingDriver>) -> Self {
        self.accounting_drivers.push(driver);
        self
    }

    pub fn build(self) -> SecurityPipeline {
        SecurityPipeline {
            auth: AuthPipeline::new(self.auth_drivers),
            authz: AuthzPipeline::new(self.authz_drivers),
            accounting: AccountingPipeline::new(self.accounting_drivers),
        }
    }
}

impl Default for SecurityPipelineBuilder {
    fn default() -> Self {
        Self::new()
    }
}
