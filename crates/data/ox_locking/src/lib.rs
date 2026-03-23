use ox_data_object::GenericDataObject;
use uuid::Uuid;

pub trait Lockable {
    fn get_gdo(&self) -> &GenericDataObject;
    fn get_gdo_mut(&mut self) -> &mut GenericDataObject;
    fn get_gdo_id(&self) -> Result<String, String>;
    fn get_lock_status(&self) -> LockStatus;
    fn set_lock_status(&mut self, status: LockStatus);
}

#[derive(Debug, Clone, PartialEq)]
pub enum LockStatus {
    Unlocked,
    Locked(Uuid),
    PendingLock(Uuid),
    PendingUnlock(Uuid),
}

pub struct LockableGenericDataObject {
    gdo: GenericDataObject,
    lock_status: LockStatus,
}

impl LockableGenericDataObject {
    pub fn new(gdo: GenericDataObject) -> Self {
        Self {
            gdo,
            lock_status: LockStatus::Unlocked,
        }
    }
}

impl Lockable for LockableGenericDataObject {
    fn get_gdo(&self) -> &GenericDataObject {
        &self.gdo
    }

    fn get_gdo_mut(&mut self) -> &mut GenericDataObject {
        &mut self.gdo
    }

    fn get_gdo_id(&self) -> Result<String, String> {
        self.gdo.get("id").ok_or_else(|| "ID not found".to_string())
    }

    fn get_lock_status(&self) -> LockStatus {
        self.lock_status.clone()
    }

    fn set_lock_status(&mut self, status: LockStatus) {
        self.lock_status = status;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lockable_gdo() {
        let mut gdo = GenericDataObject::new("id", None);
        gdo.set("name", "test");
        let mut lockable_gdo = LockableGenericDataObject::new(gdo);

        assert_eq!(lockable_gdo.get_lock_status(), LockStatus::Unlocked);
        lockable_gdo.set_lock_status(LockStatus::Locked(Uuid::new_v4()));
        assert!(matches!(lockable_gdo.get_lock_status(), LockStatus::Locked(_)));
    }
}
