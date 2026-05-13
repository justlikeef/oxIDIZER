use ox_data_object_manager::{DataDictionary, DataObjectSchema, FieldDescriptor};
use ox_type_converter::ValueType;
use thiserror::Error;

pub mod builder;
pub mod crypto;
pub mod keystore;
pub mod model;
pub mod store;
mod tests;

pub use model::*;
pub use keystore::{KeyStore, SoftwareKeyStore, open_keystore, encrypt_private_key, decrypt_private_key};
pub use store::{CertStore, OxPersistenceCertStore};
pub use builder::{CertBuilder, parse_csr, sign_csr, cross_sign_csr, cross_sign_csr_with_pem, issuer_params_from_cert_pem};

#[derive(Error, Debug)]
pub enum CertError {
    #[error("Storage error: {0}")]
    Storage(String),
    #[error("Validation error: {0}")]
    Validation(String),
    #[error("Crypto error: {0}")]
    Crypto(String),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Internal error: {0}")]
    Internal(String),
}

/// Parse a plugin config JSON string from a raw C string pointer.
/// Returns CertError::Validation if the pointer is null or the JSON is invalid.
///
/// # Safety
/// `raw` must be a valid null-terminated UTF-8 string or null.
pub unsafe fn parse_config<T: serde::de::DeserializeOwned>(
    raw: *const std::ffi::c_char,
) -> Result<T, CertError> {
    if raw.is_null() {
        return Err(CertError::Validation("null config pointer".to_string()));
    }
    let s = unsafe { std::ffi::CStr::from_ptr(raw) }
        .to_string_lossy();
    serde_json::from_str::<T>(&s)
        .map_err(|e| CertError::Validation(format!("config JSON parse: {}", e)))
}

/// Registers all ox_cert schemas with the provided DataDictionary.
pub fn register_schemas(dictionary: &mut DataDictionary) -> Result<(), CertError> {
    register_certificate_schema(dictionary)?;
    register_ssh_certificate_schema(dictionary)?;
    register_ca_key_schema(dictionary)?;
    register_ca_key_ptr_schema(dictionary)?;
    register_acme_schemas(dictionary)?;
    register_ra_schemas(dictionary)?;
    register_audit_schema(dictionary)?;
    Ok(())
}

fn register_certificate_schema(dict: &mut DataDictionary) -> Result<(), CertError> {
    let mut schema = DataObjectSchema::new("certificate");
    schema.add_field(FieldDescriptor::new("serial", ValueType::Text).primary_key());
    schema.add_field(FieldDescriptor::new("tenant_id", ValueType::Text).indexed());
    schema.add_field(FieldDescriptor::new("subject_cn", ValueType::Text));
    schema.add_field(FieldDescriptor::new("subject_dn", ValueType::Text));
    schema.add_field(FieldDescriptor::new("sans", ValueType::Json));
    schema.add_field(FieldDescriptor::new("issuer_dn", ValueType::Text));
    schema.add_field(FieldDescriptor::new("not_before", ValueType::Timestamp));
    schema.add_field(FieldDescriptor::new("not_after", ValueType::Timestamp).indexed());
    schema.add_field(FieldDescriptor::new("key_type", ValueType::Text));
    schema.add_field(FieldDescriptor::new("profile", ValueType::Text));
    schema.add_field(FieldDescriptor::new("pem", ValueType::Text));
    schema.add_field(FieldDescriptor::new("csr_pem", ValueType::Text));
    schema.add_field(FieldDescriptor::new("private_key_encrypted", ValueType::Text));
    schema.add_field(FieldDescriptor::new("status", ValueType::Text).indexed());
    schema.add_field(FieldDescriptor::new("revoked_at", ValueType::Timestamp));
    schema.add_field(FieldDescriptor::new("revocation_reason", ValueType::Integer));
    schema.add_field(FieldDescriptor::new("scts", ValueType::Json));
    schema.add_field(FieldDescriptor::new("policy_oids", ValueType::Json));
    schema.add_field(FieldDescriptor::new("enrollment_protocol", ValueType::Text));
    schema.add_field(FieldDescriptor::new("created_at", ValueType::Timestamp));
    
    dict.register_schema(schema)
        .map_err(|e| CertError::Internal(format!("Failed to register certificate schema: {}", e)))
}

fn register_ssh_certificate_schema(dict: &mut DataDictionary) -> Result<(), CertError> {
    let mut schema = DataObjectSchema::new("ssh_certificate");
    schema.add_field(FieldDescriptor::new("serial", ValueType::BigInt).primary_key());
    schema.add_field(FieldDescriptor::new("tenant_id", ValueType::Text).indexed());
    schema.add_field(FieldDescriptor::new("cert_type", ValueType::Text));
    schema.add_field(FieldDescriptor::new("key_id", ValueType::Text));
    schema.add_field(FieldDescriptor::new("principals", ValueType::Json));
    schema.add_field(FieldDescriptor::new("public_key", ValueType::Text));
    schema.add_field(FieldDescriptor::new("signing_key_fingerprint", ValueType::Text));
    schema.add_field(FieldDescriptor::new("valid_after", ValueType::Timestamp));
    schema.add_field(FieldDescriptor::new("valid_before", ValueType::Timestamp));
    schema.add_field(FieldDescriptor::new("critical_options", ValueType::Json));
    schema.add_field(FieldDescriptor::new("extensions", ValueType::Json));
    schema.add_field(FieldDescriptor::new("certificate", ValueType::Text));
    schema.add_field(FieldDescriptor::new("created_at", ValueType::Timestamp));
    
    dict.register_schema(schema)
        .map_err(|e| CertError::Internal(format!("Failed to register ssh_certificate schema: {}", e)))
}

fn register_ca_key_schema(dict: &mut DataDictionary) -> Result<(), CertError> {
    let mut schema = DataObjectSchema::new("ca_key");
    schema.add_field(FieldDescriptor::new("id", ValueType::Text).primary_key());
    schema.add_field(FieldDescriptor::new("tenant_id", ValueType::Text).indexed());
    schema.add_field(FieldDescriptor::new("key_type", ValueType::Text));
    schema.add_field(FieldDescriptor::new("cert_pem", ValueType::Text));
    schema.add_field(FieldDescriptor::new("key_ref", ValueType::Text));
    schema.add_field(FieldDescriptor::new("status", ValueType::Text));
    schema.add_field(FieldDescriptor::new("not_before", ValueType::Timestamp));
    schema.add_field(FieldDescriptor::new("not_after", ValueType::Timestamp));
    schema.add_field(FieldDescriptor::new("name_constraints", ValueType::Json));
    schema.add_field(FieldDescriptor::new("path_length", ValueType::Integer));
    schema.add_field(FieldDescriptor::new("created_at", ValueType::Timestamp));

    dict.register_schema(schema)
        .map_err(|e| CertError::Internal(format!("Failed to register ca_key schema: {}", e)))
}

/// Single-row pointer that tracks the ID of the currently-active CA signing key.
/// Primary key is "{tenant_id}:active" to avoid cross-tenant collisions.
fn register_ca_key_ptr_schema(dict: &mut DataDictionary) -> Result<(), CertError> {
    let mut schema = DataObjectSchema::new("ca_key_ptr");
    schema.add_field(FieldDescriptor::new("id", ValueType::Text).primary_key());
    schema.add_field(FieldDescriptor::new("tenant_id", ValueType::Text).indexed());
    schema.add_field(FieldDescriptor::new("data", ValueType::Text));
    dict.register_schema(schema)
        .map_err(|e| CertError::Internal(format!("Failed to register ca_key_ptr schema: {}", e)))
}

fn register_acme_schemas(dict: &mut DataDictionary) -> Result<(), CertError> {
    // Acme Account
    let mut account = DataObjectSchema::new("acme_account");
    account.add_field(FieldDescriptor::new("id", ValueType::Text).primary_key());
    account.add_field(FieldDescriptor::new("tenant_id", ValueType::Text).indexed());
    account.add_field(FieldDescriptor::new("jwk", ValueType::Text));
    account.add_field(FieldDescriptor::new("contact", ValueType::Json));
    account.add_field(FieldDescriptor::new("status", ValueType::Text));
    account.add_field(FieldDescriptor::new("eab_kid", ValueType::Text));
    account.add_field(FieldDescriptor::new("created_at", ValueType::Timestamp));
    dict.register_schema(account).map_err(|e| CertError::Internal(e.to_string()))?;

    // Acme Order
    let mut order = DataObjectSchema::new("acme_order");
    order.add_field(FieldDescriptor::new("id", ValueType::Text).primary_key());
    order.add_field(FieldDescriptor::new("tenant_id", ValueType::Text).indexed());
    order.add_field(FieldDescriptor::new("account_id", ValueType::Text).indexed());
    order.add_field(FieldDescriptor::new("status", ValueType::Text));
    order.add_field(FieldDescriptor::new("identifiers", ValueType::Json));
    order.add_field(FieldDescriptor::new("not_before", ValueType::Timestamp));
    order.add_field(FieldDescriptor::new("not_after", ValueType::Timestamp));
    order.add_field(FieldDescriptor::new("certificate_serial", ValueType::Text));
    order.add_field(FieldDescriptor::new("expires", ValueType::Timestamp));
    order.add_field(FieldDescriptor::new("created_at", ValueType::Timestamp));
    dict.register_schema(order).map_err(|e| CertError::Internal(e.to_string()))?;

    // Acme Authorization
    let mut authz = DataObjectSchema::new("acme_authorization");
    authz.add_field(FieldDescriptor::new("id", ValueType::Text).primary_key());
    authz.add_field(FieldDescriptor::new("tenant_id", ValueType::Text).indexed());
    authz.add_field(FieldDescriptor::new("order_id", ValueType::Text).indexed());
    authz.add_field(FieldDescriptor::new("identifier_type", ValueType::Text));
    authz.add_field(FieldDescriptor::new("identifier_value", ValueType::Text));
    authz.add_field(FieldDescriptor::new("status", ValueType::Text));
    authz.add_field(FieldDescriptor::new("challenges", ValueType::Json));
    authz.add_field(FieldDescriptor::new("expires", ValueType::Timestamp));
    dict.register_schema(authz).map_err(|e| CertError::Internal(e.to_string()))?;

    Ok(())
}

fn register_ra_schemas(dict: &mut DataDictionary) -> Result<(), CertError> {
    let mut ra = DataObjectSchema::new("ra_request");
    ra.add_field(FieldDescriptor::new("id", ValueType::Text).primary_key());
    ra.add_field(FieldDescriptor::new("tenant_id", ValueType::Text).indexed());
    ra.add_field(FieldDescriptor::new("csr_pem", ValueType::Text));
    ra.add_field(FieldDescriptor::new("requester_identity", ValueType::Text));
    ra.add_field(FieldDescriptor::new("profile", ValueType::Text));
    ra.add_field(FieldDescriptor::new("sans", ValueType::Json));
    ra.add_field(FieldDescriptor::new("status", ValueType::Text).indexed());
    ra.add_field(FieldDescriptor::new("reviewer", ValueType::Text));
    ra.add_field(FieldDescriptor::new("review_notes", ValueType::Text));
    ra.add_field(FieldDescriptor::new("reviewed_at", ValueType::Timestamp));
    ra.add_field(FieldDescriptor::new("certificate_serial", ValueType::Text));
    ra.add_field(FieldDescriptor::new("created_at", ValueType::Timestamp));
    
    dict.register_schema(ra)
        .map_err(|e| CertError::Internal(format!("Failed to register ra_request schema: {}", e)))
}

fn register_audit_schema(dict: &mut DataDictionary) -> Result<(), CertError> {
    let mut audit = DataObjectSchema::new("audit_log");
    audit.add_field(FieldDescriptor::new("id", ValueType::BigInt).primary_key().auto_increment());
    audit.add_field(FieldDescriptor::new("tenant_id", ValueType::Text).indexed());
    audit.add_field(FieldDescriptor::new("timestamp", ValueType::Timestamp).indexed());
    audit.add_field(FieldDescriptor::new("action", ValueType::Text));
    audit.add_field(FieldDescriptor::new("serial", ValueType::Text).indexed());
    audit.add_field(FieldDescriptor::new("actor", ValueType::Text));
    audit.add_field(FieldDescriptor::new("details", ValueType::Json));
    
    dict.register_schema(audit)
        .map_err(|e| CertError::Internal(format!("Failed to register audit_log schema: {}", e)))
}
