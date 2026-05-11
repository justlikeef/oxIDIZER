use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use crate::types::SessionToken;

#[derive(Debug)]
pub enum Credentials {
    UsernamePassword {
        username: String,
        password: SecretString,
    },
    MfaPasscode {
        session_token: SessionToken,
        code: String,
    },
    MfaPush {
        session_token: SessionToken,
    },
    BearerToken {
        token: String,
    },
    SamlAssertion {
        xml: String,
    },
    ApiKey {
        key: SecretString,
    },
    ClientCert {
        der: Vec<u8>,
    },
    KerberosTicket {
        ticket: Vec<u8>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MfaChallenge {
    PushSent { session_token: SessionToken },
    CodeRequired { session_token: SessionToken },
}
