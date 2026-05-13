use std::time::{SystemTime, UNIX_EPOCH};
use base64ct::{Base64, Encoding};

pub fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '<'  => out.push_str("&lt;"),
            '>'  => out.push_str("&gt;"),
            '&'  => out.push_str("&amp;"),
            '"'  => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

fn now_iso8601() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let days_since_epoch = secs / 86400;
    let time_of_day = secs % 86400;
    let (year, month, day) = epoch_days_to_ymd(days_since_epoch);
    let h = time_of_day / 3600;
    let m = (time_of_day % 3600) / 60;
    let s = time_of_day % 60;
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", year, month, day, h, m, s)
}

fn epoch_days_to_ymd(days: u64) -> (u32, u32, u32) {
    let mut d = days as i64;
    let mut year = 1970i32;
    loop {
        let days_in_year = if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) { 366 } else { 365 };
        if d < days_in_year { break; }
        d -= days_in_year;
        year += 1;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days: [i64; 12] = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 0usize;
    while month < 11 && d >= month_days[month] {
        d -= month_days[month];
        month += 1;
    }
    (year as u32, (month + 1) as u32, (d + 1) as u32)
}

pub fn build_metadata_xml(entity_id: &str, sso_url: &str, slo_url: &str, cert_b64: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata"
    entityID="{entity_id}">
  <md:IDPSSODescriptor WantAuthnRequestsSigned="false"
      protocolSupportEnumeration="urn:oasis:names:tc:SAML:2.0:protocol">
    <md:KeyDescriptor use="signing">
      <ds:KeyInfo xmlns:ds="http://www.w3.org/2000/09/xmldsig#">
        <ds:X509Data>
          <ds:X509Certificate>{cert_b64}</ds:X509Certificate>
        </ds:X509Data>
      </ds:KeyInfo>
    </md:KeyDescriptor>
    <md:SingleLogoutService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST"
        Location="{slo_url}"/>
    <md:SingleSignOnService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST"
        Location="{sso_url}"/>
  </md:IDPSSODescriptor>
</md:EntityDescriptor>"#,
        entity_id = xml_escape(entity_id),
        cert_b64 = cert_b64,
        sso_url = xml_escape(sso_url),
        slo_url = xml_escape(slo_url),
    )
}

pub fn build_assertion_xml(
    assertion_id: &str,
    issuer: &str,
    sp_entity_id: &str,
    acs_url: &str,
    name_id: &str,
    session_id: &str,
    ttl_secs: u64,
) -> String {
    let now = now_iso8601();
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let not_on_or_after_secs = secs + ttl_secs;
    let not_after = {
        let days = not_on_or_after_secs / 86400;
        let time = not_on_or_after_secs % 86400;
        let (y, mo, d) = epoch_days_to_ymd(days);
        format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, d, time/3600, (time%3600)/60, time%60)
    };

    format!(
        r#"<saml:Assertion xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion"
    ID="{assertion_id}" Version="2.0" IssueInstant="{now}">
  <saml:Issuer>{issuer}</saml:Issuer>
  <saml:Subject>
    <saml:NameID Format="urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress">{name_id}</saml:NameID>
    <saml:SubjectConfirmation Method="urn:oasis:names:tc:SAML:2.0:cm:bearer">
      <saml:SubjectConfirmationData NotOnOrAfter="{not_after}"
          Recipient="{acs_url}"/>
    </saml:SubjectConfirmation>
  </saml:Subject>
  <saml:Conditions NotBefore="{now}" NotOnOrAfter="{not_after}">
    <saml:AudienceRestriction>
      <saml:Audience>{sp_entity_id}</saml:Audience>
    </saml:AudienceRestriction>
  </saml:Conditions>
  <saml:AuthnStatement AuthnInstant="{now}" SessionIndex="{session_id}">
    <saml:AuthnContext>
      <saml:AuthnContextClassRef>urn:oasis:names:tc:SAML:2.0:ac:classes:Password</saml:AuthnContextClassRef>
    </saml:AuthnContext>
  </saml:AuthnStatement>
</saml:Assertion>"#,
        assertion_id = xml_escape(assertion_id),
        now = now,
        issuer = xml_escape(issuer),
        name_id = xml_escape(name_id),
        acs_url = xml_escape(acs_url),
        sp_entity_id = xml_escape(sp_entity_id),
        not_after = not_after,
        session_id = xml_escape(session_id),
    )
}

pub fn build_saml_post_form(acs_url: &str, assertion_xml: &str, relay_state: &str) -> String {
    let encoded = Base64::encode_string(assertion_xml.as_bytes());
    format!(
        r#"<!DOCTYPE html><html><body>
<form method="post" action="{acs_url}">
<input type="hidden" name="SAMLResponse" value="{encoded}"/>
{relay_state_field}
<noscript><button>Submit</button></noscript>
</form>
<script>document.forms[0].submit();</script>
</body></html>"#,
        acs_url = xml_escape(acs_url),
        encoded = encoded,
        relay_state_field = if relay_state.is_empty() {
            String::new()
        } else {
            format!(r#"<input type="hidden" name="RelayState" value="{}"/>"#, xml_escape(relay_state))
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xml_escape() {
        assert_eq!(xml_escape("<foo>&\"bar'"), "&lt;foo&gt;&amp;&quot;bar&apos;");
    }

    #[test]
    fn test_xml_escape_noop_on_plain() {
        assert_eq!(xml_escape("hello world"), "hello world");
    }

    #[test]
    fn test_metadata_xml_contains_entity_id() {
        let meta = build_metadata_xml(
            "https://idp.example.com",
            "https://idp.example.com/saml/t1/sso",
            "https://idp.example.com/saml/t1/slo",
            "CERTDATA",
        );
        assert!(meta.contains("https://idp.example.com"));
        assert!(meta.contains("CERTDATA"));
        assert!(meta.contains("EntityDescriptor"));
    }

    #[test]
    fn test_assertion_xml_escapes_name_id() {
        let xml = build_assertion_xml("id1", "https://idp.example.com",
            "urn:sp", "https://sp.example.com/acs", "<injected>", "s1", 3600);
        assert!(xml.contains("&lt;injected&gt;"));
        assert!(!xml.contains("<injected>"));
    }

    #[test]
    fn test_saml_post_form_contains_acs_and_encoded_response() {
        let assertion = "<saml:Assertion>test</saml:Assertion>";
        let form = build_saml_post_form("https://sp.example.com/acs", assertion, "state123");
        assert!(form.contains("https://sp.example.com/acs"));
        assert!(form.contains("SAMLResponse"));
        assert!(form.contains("state123"));
    }
}
