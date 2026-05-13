use crate::model::{CertificateRecord, CertStatus, EnrollmentProfile, SanType};
use crate::CertError;
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName as RcgenDN, DnType,
    ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair, KeyUsagePurpose, SanType as RcgenSan,
};
use time::OffsetDateTime;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// CertBuilder
// ---------------------------------------------------------------------------

/// Higher-level builder for X.509 certificates.
/// Builds `rcgen::CertificateParams` from our model types, then signs using rcgen.
pub struct CertBuilder {
    subject: Option<String>,
    sans: Vec<SanType>,
    validity_seconds: u64,
    is_ca: bool,
    path_length: Option<u32>,
    policy_oids: Vec<String>,
    profile: String,
}

impl CertBuilder {
    pub fn new() -> Self {
        Self {
            subject: None,
            sans: vec![],
            validity_seconds: 365 * 86400,
            is_ca: false,
            path_length: None,
            policy_oids: vec![],
            profile: "standard".to_string(),
        }
    }

    pub fn subject(mut self, dn: &str) -> Self {
        self.subject = Some(dn.to_string());
        self
    }

    pub fn sans(mut self, sans: Vec<SanType>) -> Self {
        self.sans = sans;
        self
    }

    pub fn validity_seconds(mut self, secs: u64) -> Self {
        self.validity_seconds = secs;
        self
    }

    pub fn is_ca(mut self, ca: bool, path_len: Option<u32>) -> Self {
        self.is_ca = ca;
        self.path_length = path_len;
        self
    }

    pub fn profile(mut self, name: &str) -> Self {
        self.profile = name.to_string();
        self
    }

    pub fn policy_oids(mut self, oids: Vec<String>) -> Self {
        self.policy_oids = oids;
        self
    }

    pub fn apply_profile(self, p: &EnrollmentProfile) -> Self {
        self.validity_seconds(p.validity_seconds)
            .is_ca(p.is_ca, p.path_length)
            .policy_oids(p.policy_oids.clone())
            .profile(&p.name)
    }

    /// Build `rcgen::CertificateParams` from the current builder state.
    pub fn build_params(&self) -> Result<CertificateParams, CertError> {
        let mut params = CertificateParams::default();

        let serial = Uuid::new_v4();
        params.serial_number = Some(rcgen::SerialNumber::from_slice(serial.as_bytes()));

        let now = OffsetDateTime::now_utc();
        let not_after = now + time::Duration::seconds(self.validity_seconds as i64);
        params.not_before = rcgen::date_time_ymd(now.year(), now.month() as u8, now.day());
        params.not_after = rcgen::date_time_ymd(not_after.year(), not_after.month() as u8, not_after.day());

        // Subject DN
        let mut dn = RcgenDN::new();
        if let Some(subj) = &self.subject {
            parse_dn_into(&mut dn, subj);
        }
        params.distinguished_name = dn;

        // SANs
        for san in &self.sans {
            let rcgen_san = match san {
                SanType::Dns(name) => RcgenSan::DnsName(
                    name.as_str().try_into().map_err(|_| {
                        CertError::Validation(format!("invalid DNS SAN: {}", name))
                    })?,
                ),
                SanType::Ip(addr) => RcgenSan::IpAddress(*addr),
                SanType::Email(e) => RcgenSan::Rfc822Name(
                    e.as_str().try_into().map_err(|_| {
                        CertError::Validation(format!("invalid email SAN: {}", e))
                    })?,
                ),
                SanType::Uri(u) => RcgenSan::URI(
                    u.as_str().try_into().map_err(|_| {
                        CertError::Validation(format!("invalid URI SAN: {}", u))
                    })?,
                ),
            };
            params.subject_alt_names.push(rcgen_san);
        }

        // CA / basicConstraints
        if self.is_ca {
            params.is_ca = IsCa::Ca(match self.path_length {
                Some(n) => BasicConstraints::Constrained(n.min(255) as u8),
                None => BasicConstraints::Unconstrained,
            });
            params.key_usages = vec![
                KeyUsagePurpose::DigitalSignature,
                KeyUsagePurpose::KeyCertSign,
                KeyUsagePurpose::CrlSign,
            ];
        } else {
            params.is_ca = IsCa::NoCa;
            params.key_usages = vec![
                KeyUsagePurpose::DigitalSignature,
                KeyUsagePurpose::KeyEncipherment,
            ];
            params.extended_key_usages =
                vec![ExtendedKeyUsagePurpose::ServerAuth, ExtendedKeyUsagePurpose::ClientAuth];
        }

        Ok(params)
    }

    /// Self-sign with the given key pair. Returns PEM and the `CertificateRecord`.
    pub fn self_sign(
        &self,
        tenant_id: &str,
        key_pair: &KeyPair,
    ) -> Result<CertificateRecord, CertError> {
        let params = self.build_params()?;
        let serial = Uuid::new_v4().to_string();
        let now = OffsetDateTime::now_utc();
        let not_after = now + time::Duration::seconds(self.validity_seconds as i64);

        let cert = params
            .self_signed(key_pair)
            .map_err(|e| CertError::Crypto(format!("self-sign failed: {}", e)))?;

        Ok(self.into_record(tenant_id, &serial, now, not_after, cert.pem(), self.subject.as_deref().unwrap_or("").to_string(), key_pair))
    }

    /// Sign with an issuing CA.
    /// `issuer_params` + `issuer_key` are the CA's params and key (already generated/loaded).
    pub fn sign_with_issuer(
        &self,
        tenant_id: &str,
        subject_key: &KeyPair,
        issuer_params: &CertificateParams,
        issuer_key: &KeyPair,
    ) -> Result<CertificateRecord, CertError> {
        let params = self.build_params()?;
        let serial = Uuid::new_v4().to_string();
        let now = OffsetDateTime::now_utc();
        let not_after = now + time::Duration::seconds(self.validity_seconds as i64);

        let issuer = Issuer::from_params(issuer_params, issuer_key);
        let cert = params
            .signed_by(subject_key, &issuer)
            .map_err(|e| CertError::Crypto(format!("CA signing failed: {}", e)))?;

        let issuer_dn_str = dn_to_string(&issuer_params.distinguished_name);
        Ok(self.into_record(tenant_id, &serial, now, not_after, cert.pem(), issuer_dn_str, subject_key))
    }

    fn into_record(
        &self,
        tenant_id: &str,
        serial: &str,
        now: OffsetDateTime,
        not_after: OffsetDateTime,
        pem: String,
        issuer_dn: String,
        key_pair: &KeyPair,
    ) -> CertificateRecord {
        CertificateRecord {
            serial: serial.to_string(),
            tenant_id: tenant_id.to_string(),
            subject_cn: extract_cn(self.subject.as_deref().unwrap_or("")),
            subject_dn: self.subject.clone().unwrap_or_default(),
            sans: sans_to_strings(&self.sans),
            issuer_dn,
            not_before: now,
            not_after,
            key_type: detect_key_type_str(key_pair),
            profile: self.profile.clone(),
            pem,
            csr_pem: None,
            private_key_encrypted: None,
            status: CertStatus::Active,
            revoked_at: None,
            revocation_reason: None,
            scts: vec![],
            policy_oids: self.policy_oids.clone(),
            enrollment_protocol: None,
            created_at: now,
        }
    }
}

impl Default for CertBuilder {
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// CSR signing (sign an external CSR with a CA key)
// ---------------------------------------------------------------------------

/// Sign a PEM-encoded CSR with the provided CA key pair and params.
/// If `override_sans` is Some, the CSR's SANs are replaced with the provided list
/// (enrichment/server-side SAN injection).
pub fn sign_csr(
    csr_pem: &str,
    tenant_id: &str,
    profile: &str,
    validity_seconds: u64,
    override_sans: Option<&[SanType]>,
    ca_params: &CertificateParams,
    ca_key: &KeyPair,
) -> Result<CertificateRecord, CertError> {
    use rcgen::{CertificateSigningRequestParams, SanType as RcgenSan};

    let csr_params = CertificateSigningRequestParams::from_pem(csr_pem)
        .map_err(|e| CertError::Crypto(format!("CSR parse failed: {}", e)))?;

    let now = OffsetDateTime::now_utc();
    let not_after = now + time::Duration::seconds(validity_seconds as i64);

    let mut params = csr_params.params;
    params.not_before = rcgen::date_time_ymd(now.year(), now.month() as u8, now.day());
    params.not_after = rcgen::date_time_ymd(not_after.year(), not_after.month() as u8, not_after.day());
    params.serial_number = Some(rcgen::SerialNumber::from_slice(Uuid::new_v4().as_bytes()));

    // Replace SANs if caller provided overrides.
    if let Some(sans) = override_sans {
        params.subject_alt_names.clear();
        for san in sans {
            let rcgen_san = match san {
                SanType::Dns(n) => RcgenSan::DnsName(
                    n.as_str().try_into().map_err(|_| CertError::Validation(format!("invalid DNS SAN: {}", n)))?,
                ),
                SanType::Ip(a) => RcgenSan::IpAddress(*a),
                SanType::Email(e) => RcgenSan::Rfc822Name(
                    e.as_str().try_into().map_err(|_| CertError::Validation(format!("invalid email SAN: {}", e)))?,
                ),
                SanType::Uri(u) => RcgenSan::URI(
                    u.as_str().try_into().map_err(|_| CertError::Validation(format!("invalid URI SAN: {}", u)))?,
                ),
            };
            params.subject_alt_names.push(rcgen_san);
        }
    }

    let serial = Uuid::new_v4().to_string();
    let subject_dn = dn_to_string(&params.distinguished_name);
    let subject_cn = extract_cn(&subject_dn);
    let san_strings: Vec<String> = params.subject_alt_names.iter().map(|s| match s {
        RcgenSan::DnsName(d) => d.as_ref().to_string(),
        RcgenSan::IpAddress(a) => a.to_string(),
        RcgenSan::Rfc822Name(e) => e.as_ref().to_string(),
        RcgenSan::URI(u) => u.as_ref().to_string(),
        _ => String::new(),
    }).collect();

    let issuer = Issuer::from_params(ca_params, ca_key);
    let cert = params
        .signed_by(&csr_params.public_key, &issuer)
        .map_err(|e| CertError::Crypto(format!("CSR signing failed: {}", e)))?;

    Ok(CertificateRecord {
        serial,
        tenant_id: tenant_id.to_string(),
        subject_cn,
        subject_dn,
        sans: san_strings,
        issuer_dn: dn_to_string(&ca_params.distinguished_name),
        not_before: now,
        not_after,
        key_type: "external".to_string(),
        profile: profile.to_string(),
        pem: cert.pem(),
        csr_pem: Some(csr_pem.to_string()),
        private_key_encrypted: None,
        status: CertStatus::Active,
        revoked_at: None,
        revocation_reason: None,
        scts: vec![],
        policy_oids: vec![],
        enrollment_protocol: None,
        created_at: now,
    })
}

/// Sign a CSR as a CA (intermediate) certificate, explicitly setting CA extensions.
///
/// Unlike `sign_csr`, this always sets `basicConstraints = CA:true, pathlen:1` and CA key usages
/// regardless of what the CSR requests — the issuing CA decides extensions, not the CSR.
///
/// Uses x509-parser directly for SPKI extraction rather than rcgen's CSR parsing, because
/// rcgen 0.14's reconstruction of externally-generated EC public keys produces malformed
/// SubjectPublicKeyInfo that OpenSSL rejects.
pub fn cross_sign_csr(
    csr_pem: &str,
    tenant_id: &str,
    validity_seconds: u64,
    ca_params: &CertificateParams,
    ca_key: &KeyPair,
) -> Result<CertificateRecord, CertError> {
    use rcgen::SubjectPublicKeyInfo;
    use x509_parser::prelude::FromDer;

    let now = OffsetDateTime::now_utc();
    let not_after = now + time::Duration::seconds(validity_seconds as i64);
    let serial = Uuid::new_v4();
    let issuer = Issuer::from_params(ca_params, ca_key);

    let csr_der = ::pem::parse(csr_pem)
        .map_err(|e| CertError::Crypto(format!("CSR PEM decode: {}", e)))?
        .into_contents();
    let (_, raw_csr) = x509_parser::certification_request::X509CertificationRequest::from_der(&csr_der)
        .map_err(|e| CertError::Crypto(format!("CSR DER parse: {}", e)))?;
    let subject_dn = raw_csr.certification_request_info.subject.to_string();
    let spki_raw = raw_csr.certification_request_info.subject_pki.raw;
    let spki = SubjectPublicKeyInfo::from_der(spki_raw)
        .map_err(|e| CertError::Crypto(format!("SPKI parse: {}", e)))?;
    let params = build_ca_params(&subject_dn, now, not_after, &serial);
    let cert_pem = params
        .signed_by(&spki, &issuer)
        .map_err(|e| CertError::Crypto(format!("CA signing failed: {}", e)))?
        .pem();

    let subject_cn = extract_cn(&subject_dn);
    Ok(CertificateRecord {
        serial: serial.to_string(),
        tenant_id: tenant_id.to_string(),
        subject_cn,
        subject_dn,
        sans: vec![],
        issuer_dn: dn_to_string(&ca_params.distinguished_name),
        not_before: now,
        not_after,
        key_type: "external".to_string(),
        profile: "ca_intermediate".to_string(),
        pem: cert_pem,
        csr_pem: Some(csr_pem.to_string()),
        private_key_encrypted: None,
        status: CertStatus::Active,
        revoked_at: None,
        revocation_reason: None,
        scts: vec![],
        policy_oids: vec![],
        enrollment_protocol: None,
        created_at: now,
    })
}

/// Sign a CSR as a CA (intermediate) certificate using the OpenSSL CLI.
///
/// Preferred over `cross_sign_csr` when the CA cert and key are available as PEM strings,
/// because rcgen's `DistinguishedName` uses a HashMap that deduplicates attributes sharing the
/// same OID — so `DC=justlikeef, DC=com` collapses to a single DC entry in rcgen-built certs.
/// OpenSSL preserves the full multi-valued subject from the CSR verbatim.
pub fn cross_sign_csr_with_pem(
    csr_pem: &str,
    ca_cert_pem: &str,
    ca_key_pem: &str,
    tenant_id: &str,
    validity_days: u64,
) -> Result<CertificateRecord, CertError> {
    use std::fs;
    use std::process::Command;
    use x509_parser::prelude::{FromDer, X509Certificate};

    let dir = std::env::temp_dir().join(format!("ox_cross_sign_{}", Uuid::new_v4()));
    fs::create_dir_all(&dir)
        .map_err(|e| CertError::Crypto(format!("tmp dir: {}", e)))?;

    let csr_path    = dir.join("csr.pem");
    let ca_crt_path = dir.join("ca.crt");
    let ca_key_path = dir.join("ca.key");
    let signed_path = dir.join("signed.crt");
    let ext_path    = dir.join("ext.cnf");

    let write_file = |p: &std::path::Path, d: &str| -> Result<(), CertError> {
        fs::write(p, d).map_err(|e| CertError::Crypto(format!("write {}: {}", p.display(), e)))
    };
    write_file(&csr_path,    csr_pem)?;
    write_file(&ca_crt_path, ca_cert_pem)?;
    write_file(&ca_key_path, ca_key_pem)?;
    write_file(&ext_path,    "basicConstraints=critical,CA:true,pathlen:1\nkeyUsage=critical,keyCertSign,cRLSign\n")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(&ca_key_path) {
            let mut perms = meta.permissions();
            perms.set_mode(0o600);
            let _ = fs::set_permissions(&ca_key_path, perms);
        }
    }

    let serial      = Uuid::new_v4();
    let serial_hex  = format!("0x{:032x}", serial.as_u128());

    let result: Result<CertificateRecord, CertError> = (|| {
        let out = Command::new("openssl")
            .args([
                "x509", "-req",
                "-in",         csr_path.to_str().unwrap_or(""),
                "-CA",         ca_crt_path.to_str().unwrap_or(""),
                "-CAkey",      ca_key_path.to_str().unwrap_or(""),
                "-set_serial", &serial_hex,
                "-out",        signed_path.to_str().unwrap_or(""),
                "-days",       &validity_days.to_string(),
                "-extfile",    ext_path.to_str().unwrap_or(""),
            ])
            .output()
            .map_err(|e| CertError::Crypto(format!("openssl exec: {}", e)))?;

        if !out.status.success() {
            return Err(CertError::Crypto(format!(
                "openssl x509 -req: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }

        let cert_pem = fs::read_to_string(&signed_path)
            .map_err(|e| CertError::Crypto(format!("read signed cert: {}", e)))?;

        let cert_der = ::pem::parse(cert_pem.as_bytes())
            .map_err(|e| CertError::Crypto(format!("PEM parse: {}", e)))?
            .into_contents();
        let (_, parsed) = X509Certificate::from_der(&cert_der)
            .map_err(|e| CertError::Crypto(format!("DER parse: {}", e)))?;

        let subject_dn = parsed.subject().to_string();
        let issuer_dn  = parsed.issuer().to_string();
        let subject_cn = extract_cn(&subject_dn);

        let now       = OffsetDateTime::now_utc();
        let not_after = now + time::Duration::seconds(validity_days as i64 * 86400);

        Ok(CertificateRecord {
            serial:               serial.to_string(),
            tenant_id:            tenant_id.to_string(),
            subject_cn,
            subject_dn,
            sans:                 vec![],
            issuer_dn,
            not_before:           now,
            not_after,
            key_type:             "external".to_string(),
            profile:              "ca_intermediate".to_string(),
            pem:                  cert_pem,
            csr_pem:              Some(csr_pem.to_string()),
            private_key_encrypted: None,
            status:               CertStatus::Active,
            revoked_at:           None,
            revocation_reason:    None,
            scts:                 vec![],
            policy_oids:          vec![],
            enrollment_protocol:  None,
            created_at:           now,
        })
    })();

    let _ = fs::remove_dir_all(&dir);
    result
}

fn build_ca_params(
    subject_dn: &str,
    now: OffsetDateTime,
    not_after: OffsetDateTime,
    serial: &Uuid,
) -> CertificateParams {
    use rcgen::{BasicConstraints, IsCa, KeyUsagePurpose};
    let mut params = CertificateParams::default();
    let mut dn = RcgenDN::new();
    parse_dn_into(&mut dn, subject_dn);
    params.distinguished_name = dn;
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(1));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    params.not_before = rcgen::date_time_ymd(now.year(), now.month() as u8, now.day());
    params.not_after = rcgen::date_time_ymd(not_after.year(), not_after.month() as u8, not_after.day());
    params.serial_number = Some(rcgen::SerialNumber::from_slice(serial.as_bytes()));
    params
}

/// Build rcgen `CertificateParams` suitable for use as an issuer, by extracting the subject DN
/// from an existing CA cert PEM. Used to reconstruct the issuer context without
/// `CertificateParams::from_ca_cert_pem` (not available in rcgen 0.14).
pub fn issuer_params_from_cert_pem(cert_pem: &str) -> Result<CertificateParams, CertError> {
    use x509_parser::prelude::*;

    let der = ::pem::parse(cert_pem.as_bytes())
        .map_err(|e| CertError::Crypto(format!("issuer cert PEM: {}", e)))?
        .into_contents();
    let (_, cert) = X509Certificate::from_der(&der)
        .map_err(|e| CertError::Crypto(format!("issuer cert DER: {}", e)))?;

    let subject_str = cert.subject().to_string();
    let mut params = CertificateParams::default();
    let mut dn = RcgenDN::new();
    parse_dn_into(&mut dn, &subject_str);
    params.distinguished_name = dn;
    params.is_ca = IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    Ok(params)
}

// ---------------------------------------------------------------------------
// CSR parsing
// ---------------------------------------------------------------------------

/// Parse a PEM CSR and extract subject DN, SANs, and key info.
pub fn parse_csr(csr_pem: &str) -> Result<crate::model::CsrInfo, CertError> {
    use rcgen::{CertificateSigningRequestParams, SanType as RcgenSan};

    let csr = CertificateSigningRequestParams::from_pem(csr_pem)
        .map_err(|e| CertError::Crypto(format!("CSR parse: {}", e)))?;

    let params = &csr.params;
    let subject_dn = dn_to_string(&params.distinguished_name);
    let subject_cn = extract_cn(&subject_dn);

    let sans: Vec<SanType> = params
        .subject_alt_names
        .iter()
        .map(|s| match s {
            RcgenSan::DnsName(d) => SanType::Dns(d.as_ref().to_string()),
            RcgenSan::IpAddress(a) => SanType::Ip(*a),
            RcgenSan::Rfc822Name(e) => SanType::Email(e.as_ref().to_string()),
            RcgenSan::URI(u) => SanType::Uri(u.as_ref().to_string()),
            _ => SanType::Dns(String::new()),
        })
        .collect();

    use rcgen::PublicKeyData;
    let public_key_der = csr.public_key.der_bytes().to_vec();
    let (key_type, key_bits) = detect_csr_key_type(&public_key_der);

    let raw_der = {
        use ::pem::parse as pem_parse;
        pem_parse(csr_pem)
            .map_err(|e| CertError::Crypto(format!("PEM decode: {}", e)))?
            .into_contents()
    };

    Ok(crate::model::CsrInfo {
        subject_dn,
        subject_cn,
        sans,
        key_type,
        key_bits,
        public_key_der,
        raw_der,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

// OID for domainComponent (DC): 0.9.2342.19200300.100.1.25
fn dc_dn_type() -> DnType {
    DnType::CustomDnType(vec![0, 9, 2342, 19200300, 100, 1, 25])
}

pub fn parse_dn_into(dn: &mut RcgenDN, dn_str: &str) {
    for part in dn_str.split(',') {
        let part = part.trim();
        if let Some((key, val)) = part.split_once('=') {
            match key.trim().to_uppercase().as_str() {
                "CN" => dn.push(DnType::CommonName, val.trim()),
                "O"  => dn.push(DnType::OrganizationName, val.trim()),
                "OU" => dn.push(DnType::OrganizationalUnitName, val.trim()),
                "C"  => dn.push(DnType::CountryName, val.trim()),
                "ST" => dn.push(DnType::StateOrProvinceName, val.trim()),
                "L"  => dn.push(DnType::LocalityName, val.trim()),
                // domainComponent — abbreviated, full-name, and raw OID forms.
                // x509-parser may emit the OID dotted string if the OID is not in its registry.
                "DC" | "DOMAINCOMPONENT" | "0.9.2342.19200300.100.1.25" => dn.push(dc_dn_type(), val.trim()),
                _ => {}
            }
        }
    }
}

fn extract_cn(dn_str: &str) -> String {
    for part in dn_str.split(',') {
        let part = part.trim();
        if let Some((key, val)) = part.split_once('=') {
            if key.trim().eq_ignore_ascii_case("CN") {
                return val.trim().to_string();
            }
        }
    }
    dn_str.to_string()
}

pub fn dn_to_string_pub(dn: &RcgenDN) -> String { dn_to_string(dn) }

fn dn_to_string(dn: &RcgenDN) -> String {
    use rcgen::DnValue;
    let mut parts: Vec<String> = vec![];
    for (dn_type, val) in dn.iter() {
        let key = match dn_type {
            DnType::CommonName => "CN",
            DnType::OrganizationName => "O",
            DnType::OrganizationalUnitName => "OU",
            DnType::CountryName => "C",
            DnType::StateOrProvinceName => "ST",
            DnType::LocalityName => "L",
            DnType::CustomDnType(oid) if *oid == vec![0u64, 9, 2342, 19200300, 100, 1, 25] => "DC",
            _ => "?",
        };
        let val_str = match val {
            DnValue::Utf8String(s) => s.clone(),
            DnValue::Ia5String(s) => s.as_str().to_string(),
            DnValue::PrintableString(s) => s.as_str().to_string(),
            DnValue::TeletexString(s) => s.as_str().to_string(),
            DnValue::BmpString(s) => String::from_utf8_lossy(s.as_bytes()).to_string(),
            DnValue::UniversalString(s) => String::from_utf8_lossy(s.as_bytes()).to_string(),
            _ => String::new(),
        };
        parts.push(format!("{}={}", key, val_str));
    }
    parts.join(", ")
}

fn sans_to_strings(sans: &[SanType]) -> Vec<String> {
    sans.iter()
        .map(|s| match s {
            SanType::Dns(n) => n.clone(),
            SanType::Ip(a)  => a.to_string(),
            SanType::Email(e) => e.clone(),
            SanType::Uri(u) => u.clone(),
        })
        .collect()
}

fn detect_key_type_str(kp: &KeyPair) -> String {
    let alg = kp.algorithm();
    let name = format!("{:?}", alg);
    if name.contains("Ed25519") {
        "ed25519".to_string()
    } else if name.contains("P256") || name.contains("P_256") {
        "ecc-p256".to_string()
    } else if name.contains("P384") || name.contains("P_384") {
        "ecc-p384".to_string()
    } else if name.contains("P521") || name.contains("P_521") {
        "ecc-p521".to_string()
    } else if name.to_lowercase().contains("rsa") {
        "rsa".to_string()
    } else {
        name.to_lowercase()
    }
}

fn detect_csr_key_type(spki: &[u8]) -> (crate::model::KeyType, u32) {
    const RSA_OID: &[u8]     = &[0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01];
    const EC_OID: &[u8]      = &[0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
    const P256_OID: &[u8]    = &[0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];
    const P384_OID: &[u8]    = &[0x06, 0x05, 0x2b, 0x81, 0x04, 0x00, 0x22];
    const ED25519_OID: &[u8] = &[0x06, 0x03, 0x2b, 0x65, 0x70];

    fn has(h: &[u8], n: &[u8]) -> bool { h.windows(n.len()).any(|w| w == n) }

    use crate::model::KeyType;
    if has(spki, ED25519_OID) { return (KeyType::Ed25519, 256); }
    if has(spki, RSA_OID) {
        let bits = if spki.len() > 512 { 4096u32 } else if spki.len() > 384 { 3072 } else { 2048 };
        let kt = match bits { 4096 => KeyType::Rsa4096, 3072 => KeyType::Rsa3072, _ => KeyType::Rsa2048 };
        return (kt, bits);
    }
    if has(spki, EC_OID) {
        if has(spki, P256_OID) { return (KeyType::EcP256, 256); }
        if has(spki, P384_OID) { return (KeyType::EcP384, 384); }
        return (KeyType::EcP521, 521);
    }
    (KeyType::EcP256, 256)
}
