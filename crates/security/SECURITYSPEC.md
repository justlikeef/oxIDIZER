# ox_security — Security System Design Spec

## Overview

A generic AAA (Authentication, Authorization, Accounting) pipeline for oxIDIZER. Designed initially as an internal AAA consumer; architected to become a federated AAA gateway (SAML IdP, OAuth2 authorization server) that aggregates multiple identity and permission sources — modelled on FortiIdentity.

The security system is not opinionated about permission structure or operation vocabulary. Consuming crates register their own context trees and operations. The security system evaluates whatever is registered. The data crates are used as the universal persistence abstraction — all IAM entities (principals, groups, grants, sessions, audit events) are stored and retrieved through the data crate layer regardless of the backend (SQL, LDAP, AD, Okta, etc.).

---

## Crate Structure

```
crates/security/
  ox_security_core/         shared types, driver traits, registration API
  ox_security_auth/         authentication pipeline + drivers
  ox_security_authz/        authorization pipeline + drivers
  ox_security_accounting/   audit/logging pipeline + drivers
  ox_security_pipeline/     composes the three into one webservice plugin
```

Future crate (not in scope for this spec):
```
  ox_security_idp/          SAML IdP + OAuth2 authorization server (gateway mode)
```

---

## Core Types (`ox_security_core`)

### Credentials
Covers all authentication paths — the auth driver chain inspects whichever variant is present.

```rust
pub enum Credentials {
    UsernamePassword { username: String, password: SecretString },
    MfaPasscode      { session_token: SessionToken, code: String },
    MfaPush          { session_token: SessionToken },
    BearerToken      { token: String },          // OIDC / OAuth2 access token
    SamlAssertion    { xml: String },
    ApiKey           { key: SecretString },
    ClientCert       { der: Vec<u8> },           // mTLS — DER-encoded client cert
    KerberosTicket   { ticket: Vec<u8> },
}
```

### Principal
Authenticated identity — user, service account, or federated subject. Group membership is resolved by the auth driver at authentication time and embedded here; the authz pipeline reads it directly without re-querying the identity source.

```rust
pub struct Principal {
    pub id: PrincipalId,
    pub display_name: String,
    pub source: AuthSource,       // Local | Ldap | Ad | Okta | Saml | Radius | ...
    pub groups: Vec<GroupId>,     // resolved and normalised at auth time
    pub tenant_id: TenantId,
    pub session_id: Option<SessionId>,
}
```

### SecurityContext
Carried on every request through the pipeline. Applications set `call_context` before passing the context to any object they use. Objects call `check` with only their own fragment — the pipeline prepends the call context.

```rust
pub struct SecurityContext {
    pub principal: Option<Principal>,  // None until authenticated
    pub call_context: String,          // set by the application before invoking objects
    pub tenant_id: TenantId,
}

impl SecurityContext {
    // Called by objects using only their own fragment.
    // Pipeline resolves: call_context + "." + object_fragment -> full path -> evaluates grants.
    pub async fn check(&self, object_fragment: &str, operation: &str) -> Result<(), AuthzError>;
}
```

### OperationDef
Operations are defined by the crate that registers the context node where they apply. `ox_security_core` provides well-known names as constants; domain crates define their own alongside their context registrations.

```rust
pub struct OperationDef {
    pub name: &'static str,        // e.g. "read", "issue", "revoke", "ddl"
    pub description: &'static str, // shown in admin UI
}

// Well-known operations provided by ox_security_core:
pub const OP_READ:    OperationDef = OperationDef { name: "read",    description: "Read a value or record" };
pub const OP_WRITE:   OperationDef = OperationDef { name: "write",   description: "Write a value or record" };
pub const OP_CREATE:  OperationDef = OperationDef { name: "create",  description: "Create a new record" };
pub const OP_CHANGE:  OperationDef = OperationDef { name: "change",  description: "Modify an existing record" };
pub const OP_DELETE:  OperationDef = OperationDef { name: "delete",  description: "Delete a record" };
pub const OP_LIST:    OperationDef = OperationDef { name: "list",    description: "List or enumerate records" };
pub const OP_EXECUTE: OperationDef = OperationDef { name: "execute", description: "Execute a function or procedure" };
pub const OP_DDL:     OperationDef = OperationDef { name: "ddl",     description: "Modify schema or structure" };
```

Domain crates define additional operations specific to their context:

```rust
// In ox_cert:
pub const OP_ISSUE:  OperationDef = OperationDef { name: "issue",  description: "Issue a new certificate" };
pub const OP_RENEW:  OperationDef = OperationDef { name: "renew",  description: "Renew an existing certificate" };
pub const OP_REVOKE: OperationDef = OperationDef { name: "revoke", description: "Revoke a certificate" };
pub const OP_SIGN:   OperationDef = OperationDef { name: "sign",   description: "Sign a CSR" };
pub const OP_EXPORT: OperationDef = OperationDef { name: "export", description: "Export certificate or key material" };
```

### SecurityRegistration
Objects that participate in the permission model implement this trait to expose their own context fragment. They know nothing about which application uses them — they only describe themselves.

```rust
pub trait SecurityRegistration {
    fn context_definition(&self) -> ContextDefinition;
}
```

The application queries its objects via this trait at startup and wraps each definition under its own path prefix before registering with the pipeline. The same object can be registered under multiple application branches with identical definitions — grants on each branch are evaluated independently.

### Driver Traits

```rust
pub trait AuthDriver: Send + Sync {
    async fn authenticate(
        &self,
        credentials: &Credentials,
        ctx: &mut AuthPipelineContext,
    ) -> AuthResult;
}

// AuthPipelineContext carries state across the driver chain for a single auth attempt:
pub struct AuthPipelineContext {
    pub partial_principal: Option<PartialPrincipal>, // set by credential driver, read by MFA driver
    pub tenant_id: TenantId,
    pub source_ip: IpAddr,
}

pub enum AuthResult {
    Authenticated(Principal),         // auth complete, proceed
    MfaRequired(MfaChallenge),        // credential verified, MFA step needed
    Continue,                         // this driver does not handle these credentials, try next
    Reject(String),                   // explicit rejection, stop chain
}

pub enum MfaChallenge {
    PushSent { session_token: SessionToken },   // Duo push dispatched
    CodeRequired { session_token: SessionToken }, // TOTP or passcode expected
}

pub trait AuthzDriver: Send + Sync {
    async fn check(
        &self,
        principal: &Principal,
        path: &str,          // full resolved path (call_context + "." + object_fragment)
        operation: &str,     // operation name, e.g. "read", "issue"
    ) -> AuthzResult;
    // AuthzResult: Allow | Deny(reason)
}

pub trait AccountingDriver: Send + Sync {
    async fn record(&self, event: &AccountingEvent);
}
```

---

## Context & Permission Registration

Consuming crates declare their context tree fragments at startup via a `ContextRegistrar` reference provided by `ox_security_pipeline` during initialisation. The security system persists registrations via the data crates and uses them to validate grants and populate admin UIs. The security system imposes no opinion on structure or naming.

```rust
pub struct ContextDefinition {
    pub root: &'static str,
    pub operations: &'static [OperationDef], // operations defined AT this specific node
    pub children: &'static [ContextDefinition],
}

pub trait ContextRegistrar {
    fn register_context(&self, def: ContextDefinition);
}
```

The `SecurityPipeline` implements `ContextRegistrar`. A reference to the pipeline is passed to each consuming crate's `register` initialisation function at startup.

### Operation Scope Rules

**Registration** — `operations` on a `ContextDefinition` declares which operations exist *at that specific node*. Operations are defined by the domain crate at the nodes where they are meaningful.

**Grantability bubbles up** — any operation registered anywhere in a subtree can be granted at any ancestor of that subtree. A grant at an ancestor cascades down and takes effect at every descendant where that operation is registered.

**Evaluation cascades down** — a grant at node X for operation Y applies to all descendants of X that have Y registered, subject to the specificity and most-permissive rules.

```
Example — cert domain registers:
  com.justlikeef.application.cert_admin          [read, list]
    certificates                                 [read, list, issue, renew, revoke, sign, export]
    ca                                           [read, sign, ddl]

Valid grant operations at com.justlikeef.application.cert_admin:
  [read, list, issue, renew, revoke, sign, export, ddl]   <- union of entire subtree

Valid grant operations at com.justlikeef:
  [read, list, issue, renew, revoke, sign, export, ddl, ...] <- union of all registered subtrees

Granting "issue" at com.justlikeef.application.cert_admin cascades down and takes
effect only at nodes where "issue" is registered (certificates). It has no effect
at ca (issue not registered there) or at the root node itself.
```

### Example Registrations

Objects implement `SecurityRegistration` to describe themselves. They know only their own fragment.

```rust
// dataobject1 describes itself — no knowledge of which application uses it:
impl SecurityRegistration for DataObject1 {
    fn context_definition(&self) -> ContextDefinition {
        ContextDefinition {
            root: "dataobject1",
            operations: &[],
            children: &[
                ContextDefinition {
                    root: "field1",
                    operations: &[OP_READ, OP_WRITE, OP_CHANGE, OP_DELETE],
                    children: &[],
                },
                ContextDefinition {
                    root: "field2",
                    operations: &[OP_READ],
                    children: &[],
                },
            ],
        }
    }
}
```

Applications query their objects at startup and register them under their own branch:

```rust
// The data application registers dataobject1 under com.justlikeef.data:
pipeline.register_context(ContextDefinition {
    root: "com.justlikeef.data",
    operations: &[],
    children: &[dataobject1.context_definition()],
});

// application1 independently registers dataobject1 under its own branch:
pipeline.register_context(ContextDefinition {
    root: "com.justlikeef.application1",
    operations: &[],
    children: &[dataobject1.context_definition()],
});

// ox_cert registers its own context — completely different operation vocabulary:
pipeline.register_context(ContextDefinition {
    root: "com.justlikeef.application.cert_admin",
    operations: &[OP_READ, OP_LIST],
    children: &[
        ContextDefinition {
            root: "certificates",
            operations: &[OP_READ, OP_LIST, OP_ISSUE, OP_RENEW, OP_REVOKE, OP_SIGN, OP_EXPORT],
            children: &[],
        },
        ContextDefinition {
            root: "ca",
            operations: &[OP_READ, OP_SIGN, OP_DDL],
            children: &[],
        },
    ],
});
```

### Check-Time Pattern

The application sets `call_context` on the `SecurityContext` before passing it to any object. Objects call `ctx.check` with only their own fragment — they never see the full path.

```rust
// Application sets call context before invoking the object:
ctx.call_context = "com.justlikeef.application1".to_string();
dataobject1.do_change(value, &ctx).await?;

// Inside dataobject1.do_change — no knowledge of application1:
impl DataObject1 {
    pub async fn do_change(&self, value: Value, ctx: &SecurityContext) -> Result<()> {
        ctx.check("dataobject1.field1", "change").await?;
        // pipeline resolves: "com.justlikeef.application1.dataobject1.field1"
        // evaluates grants at the full path
        self.field1 = value;
        Ok(())
    }
}
```

The same object under the data application uses `call_context = "com.justlikeef.data"` — the pipeline resolves a different full path and evaluates against different grants with no change to the object's code.

Granting `read` at `com.justlikeef` covers every node in the entire registered tree that has `read` defined — all from a single grant at the root.

---

## Permission Model

### Hierarchical Context Tree

Permissions are assigned to nodes in a path hierarchy:

```
com.justlikeef
  data                                    <- grant here cascades to all data.*
    dataobject1
      field1
      field2
    dataobject2
      field1
  application.cert_admin                  <- grant here cascades to all cert_admin.*
    certificates
    ca
  application1                            <- grant here cascades to all application1.*
    dataobject1                           <- grant here is specific to this branch only
      field1
      field2
```

### Grant Records

Each node carries zero or more grants: `(group, operation_name, Allow | Deny)`.

### Two-Part Context

Every permission check carries two fragments combined by the pipeline before evaluation:

- **Call context** — set by the application on `SecurityContext` before invoking any object (e.g., `com.justlikeef.application1`, `com.justlikeef.data`)
- **Object fragment** — passed by the object to `ctx.check()` using only its own identity (e.g., `dataobject1.field1`)
- **Resolved path** — `call_context + "." + object_fragment`, assembled by the pipeline

Objects are entirely context-unaware — they call `ctx.check("my.fragment", "operation")` and the pipeline resolves the full path. The same object under different applications resolves to different paths and evaluates against different grants with no change to object code. This resolves the ambiguity where `dataobject1` appears in multiple branches — `application1.dataobject1` and `application2.dataobject1` are distinct paths with independently configured grants.

### Evaluation Algorithm

Given `(principal, call_context, object_fragment, operation)`:

1. Resolve full path: `call_context + "." + object_fragment`
2. Collect all path ancestors (including self), ordered most-specific to root
3. For each of the principal's groups, find the deepest ancestor node that has a grant for that group and operation
4. **Most specific wins** — grants at shallower depth are ignored for a group if a deeper grant exists for that group
5. **Same depth, multiple groups** — take the union (most permissive): a single `Allow` from any group grants access; `Deny` is effective only when no group has `Allow` at that same depth

### Examples

```
Group assignments:
  dataadmins: read/write/create/change/delete/ddl  at  com.justlikeef.data
  executive:  read                                 at  com.justlikeef.data
  finance:    read                                 at  com.justlikeef.application1
  finance:    read/write/create/change/delete       at  com.justlikeef.application2
  finance:    read/write                            at  com.justlikeef.application3
  it:         read/write/create/change/delete       at  com.justlikeef.application3
  it:         read/write                            at  com.justlikeef.application2.dataobject1.field1
  it:         read/write/create/change/delete       at  com.justlikeef.application1.dataobject1

User assignments:
  bob   -> dataadmins
  nancy -> finance
  john  -> it

Checks:
  bob   change  com.justlikeef.data > dataobject2.field1               -> ALLOW (dataadmins at data, cascades down)
  john  write   com.justlikeef.application1 > dataobject1.field1       -> ALLOW (it at application1.dataobject1, cascades to field1)
  john  write   com.justlikeef.application2 > dataobject1.field1       -> ALLOW (it explicit rw grant at application2.dataobject1.field1)
  john  change  com.justlikeef.application2 > dataobject1.field1       -> DENY  (it grant at application2.dataobject1.field1 is rw only)
  nancy read    com.justlikeef.application1 > dataobject1.field1       -> ALLOW (finance read at application1, cascades down)
  nancy write   com.justlikeef.application1 > dataobject1.field1       -> DENY  (finance has read only at application1)
```

---

## Data Model & IAM Normalisation

### Canonical Schema

The security crates define a canonical IAM schema. This is the only representation the security layer ever works with — it has no knowledge of how or where data is persisted. The data crates are solely responsible for translating this canonical schema to and from the native format of the configured backend (SQL, LDAP DIT, Active Directory, Okta, or any other supported store).

| Entity | Fields |
|---|---|
| `PrincipalRecord` | `principal_id`, `display_name`, `source`, `tenant_id` |
| `SecurityGroup` | `group_id`, `name`, `source`, `tenant_id` |
| `GroupMember` | `group_id`, `principal_id`, `tenant_id` |
| `PermissionNode` | `path`, `parent_path`, `tenant_id` |
| `PermissionGrant` | `node_path`, `group_id`, `operation_name`, `allow_deny`, `tenant_id` |
| `SessionRecord` | `session_id`, `principal_id`, `expires_at`, `tenant_id` |
| `ContextRegistration` | `path`, `registering_crate`, `tenant_id` |
| `ContextOperation` | `path`, `operation_name`, `description`, `registering_crate`, `tenant_id` |
| `AuditEvent` | `principal_id`, `auth_outcome`, `authz_outcome`, `resolved_path`, `operation_name`, `timestamp`, `source_ip`, `session_id`, `tenant_id` |

### Data Layer Responsibility

The data layer translates the canonical schema to the backend's native format and back. How much of the schema a given backend can natively represent is a data layer concern — the security layer always reads and writes the same canonical types regardless.

Examples of translation:
- **SQL backend** — entities map directly to tables; all entities fully supported
- **LDAP backend** — `PrincipalRecord` maps to directory entries; `GroupMember` maps to `memberOf`; `PermissionGrant` maps to custom attributes or auxiliary object classes if the schema supports them; fields with no native LDAP equivalent are stored in a local overflow table managed by the data crate
- **AD backend** — principals and groups map to directory objects; grants map to custom AD attributes or group policy extensions where supported
- **Okta backend** — principals map to Okta users; groups map to Okta groups; grants map to custom roles or profile attributes where supported

The security layer calls the same data crate API for every backend. Backend-specific translation, partial capability handling, and any local overflow storage are invisible to the security crates.

### Normalisation at Authentication Time

Auth drivers read identity and group membership from the external source and write canonical `PrincipalRecord`, `SecurityGroup`, and `GroupMember` entities via the data crates. The resulting `Principal.groups` embedded in the session is always the normalised internal representation. The authz pipeline reads canonical entities at check time — it never queries the external source directly.

---

## Authentication Pipeline (`ox_security_auth`)

Drivers are stacked in configured order. Each returns `Authenticated`, `MfaRequired`, `Continue`, or `Reject`.

### Local Credential Drivers
| Driver | Protocol | Notes |
|---|---|---|
| `LdapAuthDriver` | LDAP bind | resolves `memberOf` into groups at auth time |
| `AdAuthDriver` | Active Directory NTLM / Kerberos | resolves AD group membership at auth time |
| `KerberosAuthDriver` | Kerberos ticket validation | |
| `DbAuthDriver` | Local credential store via data crates | |
| `RadiusAuthDriver` | RADIUS Access-Request | |
| `TacacsAuthDriver` | TACACS+ authentication | |

### MFA Drivers
Run after a credential driver returns `Authenticated` with a partial principal. The pipeline holds the partial principal in `AuthPipelineContext` and re-enters the chain with the MFA credentials.

| Driver | Protocol |
|---|---|
| `DuoAuthDriver` | Duo Security push / passcode |
| `TotpAuthDriver` | RFC 6238 TOTP (authenticator apps) |

MFA flow:
1. Credential driver returns `Authenticated(partial_principal)` — stored in `AuthPipelineContext`
2. Pipeline checks config: is MFA required for this source?
3. If yes — MFA driver sends challenge, returns `MfaRequired(MfaChallenge)` to caller
4. Caller re-submits with `Credentials::MfaPasscode` or `Credentials::MfaPush`
5. MFA driver verifies, returns `Authenticated(full_principal)`

Federated drivers that handle MFA externally skip steps 2–5 entirely.

### Federated Drivers (provider handled credential + MFA)
| Driver | Protocol |
|---|---|
| `OidcAuthDriver` | OpenID Connect / OAuth2 (Okta, Entra, etc.) |
| `SamlSpAuthDriver` | SAML 2.0 Service Provider |

### Service/API Drivers (stateless, per-request)
| Driver | Protocol |
|---|---|
| `ApiKeyAuthDriver` | Static or rotating API key in request header |
| `MtlsAuthDriver` | Mutual TLS client certificate chain validation |

### Session Handling

- **Browser principals**: server-side `SessionRecord` issued after auth; stored via data crates; delivered as a secure `HttpOnly` cookie; TTL configured per auth source
- **Federated principals**: session TTL mirrors inbound token expiry; `SessionRecord` mirrors the token's `exp` claim
- **Service/API principals**: stateless — auth driver chain runs on every request (fast path, no session record)

---

## Authorization Pipeline (`ox_security_authz`)

### Drivers
| Driver | Backend |
|---|---|
| `LocalDbAuthzDriver` | SQL database via data crates |
| `LdapAuthzDriver` | LDAP directory via data crates |
| `AdAuthzDriver` | Active Directory via data crates |
| `OktaAuthzDriver` | Okta via data crates |

All drivers read and write the same canonical IAM schema via the data crate API. How much of the schema the underlying backend can natively hold is handled transparently by the data crate — the authz driver is unaware of it.

### Strictness Mode (configurable)
- **Strict**: grants referencing paths not in `ContextRegistration` are rejected at startup
- **Permissive**: unregistered paths are allowed (useful during migration or development)

---

## Accounting Pipeline (`ox_security_accounting`)

Fire-and-forget from the request path. Driver failures are logged but never fail the request. Always runs after the response is determined, whether the request succeeded or failed.

### AccountingEvent

```rust
pub struct AccountingEvent {
    pub principal_id: Option<PrincipalId>,
    pub auth_outcome: AuthOutcome,
    // AuthOutcome: Authenticated | Failed(reason) | MfaRequired | MfaFailed
    pub authz_outcome: Option<AuthzOutcome>,
    // AuthzOutcome: Allowed | Denied(path, operation_name)
    pub call_context: String,
    pub object_fragment: Option<String>,
    pub operation_name: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub source_ip: IpAddr,
    pub session_id: Option<SessionId>,
    pub tenant_id: TenantId,
}
```

### Drivers
| Driver | Destination |
|---|---|
| `DbAccountingDriver` | Local audit table via data crates; queryable |
| `SyslogAccountingDriver` | CEF / RFC 5424 to syslog endpoint |
| `FileAccountingDriver` | Structured JSON log (SIEM forwarding) |
| `TacacsAccountingDriver` | TACACS+ accounting records |

---

## Webservice Integration (`ox_security_pipeline`)

Single integration point. The webservice registers the `SecurityPipeline` as one plugin at startup. The pipeline implements `ContextRegistrar` — consuming crates call `pipeline.register_context(...)` during their own initialisation.

The pipeline sequences on every request: **authenticate → authorize → (handler) → account**.

The webservice creates a `SecurityContext` per request with `principal = None` and an empty `call_context`. After authentication, `principal` is populated. The handler sets `call_context` to its application branch before invoking any objects. Objects call `ctx.check(fragment, operation)` — the pipeline appends the fragment to the call context, resolves the full path, and evaluates against the configured authz drivers.

---

## Multi-Tenancy

All data entities carry `tenant_id`. Grant evaluation, context registration, group membership synchronisation, and audit events are all tenant-scoped. Mirrors the pattern used in `ox_cert_core`.

---

## Long-Term: Gateway Mode (`ox_security_idp`)

When `ox_security_idp` is added, the pipeline gains the ability to issue SAML assertions and OAuth2/OIDC tokens outbound. External systems authenticate against `ox_security` as the IdP. The security system aggregates its configured AAA backends and presents a unified identity surface to the outside world — operating as a FortiIdentity-style AAA gateway. The driver trait boundaries established here are designed so this flip (consumer to provider) does not require rearchitecting the core.
