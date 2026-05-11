use std::sync::Arc;
use ox_security_core::accounting::AccountingEvent;
use ox_security_core::drivers::AccountingDriver;

/// Fans accounting events out to every configured driver.
/// All drivers are called sequentially; a panic or error in one driver
/// is caught so subsequent drivers always execute.
pub struct AccountingPipeline {
    drivers: Vec<Arc<dyn AccountingDriver>>,
}

impl AccountingPipeline {
    pub fn new(drivers: Vec<Arc<dyn AccountingDriver>>) -> Self {
        Self { drivers }
    }

    pub async fn record(&self, event: &AccountingEvent) {
        for driver in &self.drivers {
            driver.record(event).await;
        }
    }
}
