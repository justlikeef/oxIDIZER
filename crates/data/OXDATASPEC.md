# ox_data — Data Services Module

The data services module provides a generic, extensible data object model. At its core is
`GenericDataObject` — a pure data container responsible only for storing typed attributes,
converting between types, and firing callbacks on mutations. Everything else (persistence,
transactions, validation, cross-datasource queries, remote access) is an addon system that
builds on top of the core without modifying it.

`ox_locking` should be tenamed `ox_transaction` to handle transactions, rollbacks, etc. All transactions are managed with a Transaction Manager and multiple transactions can be run in parallel. It should also include the ability to define transaction boundaries and commit or rollback transactions.

`ox_transaction_manager` in crates/data/ox_transaction_manager should handle the management of transactions. It should allow the admin to view transactions, lock them, unlock them, and commit or rollback transactions. The transaction manager should be thread safe and allow multiple transactions to be run in parallel.

---

## Architecture
- `ox_callbacks_manager` in crates/util/ox_callbacks_manager 
- `ox_introspectioN` is to allow deep introspection into the data object include fields, field types, rules, relationships, etc. so that other packages can introspect the data objects to do things like build automated or manual forms, bind data to the fields, auto check data rules client side and server side, etc."
- "ox_data_broker is a plugin for ox_webservice that will provide a REST GraphQL CRUD interface to the data object and marshal access to them for multiple users through the transactional capabilities."
- `ox_transaction` to ensure data integrity for concurrent users.
```
┌─────────────────────────────────────────────────────────────────┐
│                       Application Code                          │
└──┬─────────────┬──────────────┬───────────────┬────────────────┘
   │             │              │               │
   ▼             ▼              ▼               ▼
Persistent   Transactable  Validatable   DataObjectManager
(addon)      (addon)        (addon)        (addon)
   │             │              │               │
   └─────────────┴──────────────┴───────────────┘
                         │ all addons operate on
                         ▼
┌─────────────────────────────────────────────────────────────────┐
│                  GenericDataObject                              │
│  attributes: HashMap<String, AttributeValue>  ← typed data     │
│  extensions: HashMap<String, serde_json::Value> ← addon state  │
│  callbacks:  CallbackManager  ← per-object hooks               │
│  identifier_name: String                                        │
└──────────────────────────┬──────────────────────────────────────┘
                           │
          ┌────────────────┴────────────────┐
          ▼                                 ▼
┌─────────────────────┐         ┌────────────────────────┐
│  ox_type_converter  │         │     ox_persistence     │
│  ValueType          │         │  PersistenceDriver tr. │
│  ConversionRegistry │         │  PERSISTENCE_DRIVER_   │
│  TypeConverter      │         │  REGISTRY              │
└─────────────────────┘         └──────────┬─────────────┘
                                           │
                   ┌───────────────────────┼──────────────────────┐
                   ▼                       ▼                      ▼
        ┌──────────────────┐  ┌────────────────────────┐  ┌──────────────┐
        │  File/JSON/YAML  │  │  ox_persistence_gdo_   │  │  SQL drivers │
        │  drivers         │  │  relational (wrapper)  │  │  pg/sqlite/… │
        └──────────────────┘  └────────────────────────┘  └──────────────┘

┌─────────────────────────────────────────────────────────────────┐
│                  ox_data_object_manager                         │
│  DataDictionary   DataObjectManager   QueryEngine               │
│  cross-datasource joins, attribute remapping, cardinality       │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│  ox_workflow Plugins (cdylib)                                   │
│  ox_data_broker                — REST API for data ops         │
│  ox_data_object_dictionary_manager — dictionary CRUD REST API  │
│  ox_persistence_datasource_manager — datasource CRUD           │
│  ox_persistence_driver_manager     — driver lifecycle          │
│  ox_persistence_driver_installer   — package installation      │
└─────────────────────────────────────────────────────────────────┘
```

---

## Design Principles

**GDO is data only.** `GenericDataObject` stores named, typed values. It knows nothing
about persistence, locking, workflows, or any domain concept. Addon systems receive a GDO,
operate on it, and store their own state in its extension slot — the GDO carries that data
but never interprets it.

**Addons, not wrappers.** Persistence, transactions, validation, and other capabilities
are separate systems that work with `GenericDataObject` instances. They use the object's
extension slot to attach their own state so no combination of capabilities requires a
purpose-built wrapper struct.

**Universal callbacks.** Every data manipulation — in the GDO core and in every addon —
fires `before_*` / `after_*` / `on_error_*` events through the GDO's two-level callback
dispatch: per-object callbacks first, then the global `CALLBACK_MANAGER`.

**Extensions are serialized.** The extension slot round-trips through
`to_serializable_map()` / `from_serializable_map()` as opaque JSON. Addon state (such as
which driver an object came from, or its lock status) survives serialization.

**Transactions wrap locking.** When a transaction is active on a GDO, multi-container
saves are atomic: all succeed or all are rolled back. Without an active transaction,
failures are logged and execution continues. Transactions are:
- managed by ox_transaction
- should be used for all data operations
- should mimic behavior of ACID transactions
- should be used for all cross-container saves
- should use optimistic locking
- use the datastore transaction capability if possible

**Drivers are FFI plugins.** Persistence drivers are dynamically loaded shared libraries
(`.so`/`.dylib`/`.dll`) that export a standard C ABI.

**Type safety across boundaries.** `ox_type_converter` provides a central registry of
conversions between any two `ValueType`s. `GenericDataObject::get<T>` automatically
converts stored values to the requested Rust type.

---

## Component Specs

| Spec | Contents |
|------|----------|
| [spec/core.md](spec/core.md) | `GenericDataObject`, `AttributeValue`, callbacks, extension slot, `Introspectable` trait |
| [spec/types.md](spec/types.md) | `ValueType`, `TypeConverter`, `ConversionRegistry` |
| [spec/persistence.md](spec/persistence.md) | `PersistenceDriver`, `Persistent`, `DataObjectState`, `discard` |
| [spec/transactions.md](spec/transactions.md) | `Transactable`, `LockStatus`, force unlock, TTL, transaction semantics |
| [spec/validation.md](spec/validation.md) | `ValidationRule`, built-in rules, `ValidationSet`, `Validatable` |
| [spec/dictionary.md](spec/dictionary.md) | `DataDictionary`, `DataObjectManager`, cardinality, relationships |
| [spec/query.md](spec/query.md) | `QueryEngine`, `QueryPlan`, `QueryNode`, join execution, filters |
| [spec/introspection.md](spec/introspection.md) | `ox_introspection`, `ObjectSchema`, `FieldDescriptor`, forms integration |
| [spec/drivers.md](spec/drivers.md) | Driver FFI ABI (incl. `ox_driver_delete`), management plugins |
| [spec/dictionary_manager.md](spec/dictionary_manager.md) | `ox_data_object_dictionary_manager` REST plugin |
| [spec/broker.md](spec/broker.md) | `ox_data_broker` REST API, pagination, delete endpoint |

---

## Crate Map

| Crate | Type | Depends On |
|-------|------|-----------|
| `ox_type_converter` | lib | — |
| `ox_callback_manager` | lib | — |
| `ox_data_object` | lib | `ox_type_converter`, `ox_callback_manager` |
| `ox_persistence` | lib | `ox_data_object`, `ox_type_converter` |
| `ox_transaction` | lib | `ox_data_object`, `ox_persistence`, `ox_callback_manager` |
| `ox_validation` | lib | `ox_data_object`, `ox_callback_manager` |
| `ox_data_object_manager` | lib | `ox_data_object`, `ox_persistence`, `ox_type_converter` |
| `ox_introspection` | lib | `ox_data_object`, `ox_data_object_manager`, `ox_validation` |
| `ox_data_object_dictionary_manager` | cdylib plugin | `ox_data_object_manager`, `ox_workflow_abi` |
| `ox_persistence_gdo_relational` | cdylib driver | `ox_persistence`, `ox_data_object`, `ox_type_converter` |
| `ox_persistence_driver_manager` | cdylib plugin | `ox_persistence`, `ox_workflow_abi` |
| `ox_persistence_driver_installer` | cdylib plugin | `ox_persistence`, `ox_fileproc`, `ox_workflow_abi` |
| `ox_persistence_datasource_manager` | cdylib plugin | `ox_persistence`, `ox_data_object_manager`, `ox_workflow_abi`, `ox_forms_api` |
| `ox_data_broker` | cdylib plugin | `ox_persistence`, `ox_transaction`, `ox_callback_manager`, `ox_workflow_abi`, `ox_fileproc` |
|`ox_data_broker`| cdylub plugin | `ox_webservice`|

Sample Workflow:
Install and enable driver
configure datasource
introspect datasource to get field definition
store in data dictionary
-- rich field definitions to help users understand the data
-- relationships between data within each datastore and between datastores
	--- should support multiple join strategies
-assemble data objects from field definitions
-- stored in data dictionary
-- primary datastore/datastore container for data object as base for hydrating data
-- data objects can contain fields from multiple datastores
-rich description of dataobject and usage
-define data rules
instantiate instance of dataobject by definition name
set values of field to use for filtering list or hydrating single object
get paginated list or hydrate single object
work with data
- Form built from meta information or manual
-- Bind attributes to form fields
-- Client side data rules built from meta information
-- Multiuser change monitoring and real time updates using websockets
-- submit changes
- apply data rules server side

Key Design Decisions:
- OxDataError is the central error type in ox_data_object.
- ox_callback_manager has its own CallbackError which is then wrapped by OxDataError in the data object.
- Error handling rule: Never swallow errors; propagate them using Result.

---

## Requirements for IAM / Security Layer Support

The `ox_security` crates require the data layer to persist and retrieve a canonical IAM schema across heterogeneous backends (SQL, LDAP, Active Directory, Okta, RADIUS, TACACS+). The data layer is solely responsible for translating the canonical schema to and from each backend's native format — the security crates never see backend-specific representations.

The following work is required before the security layer can be fully delivered.

### 1. New Persistence Drivers

| Driver Crate | Backend | Notes |
|---|---|---|
| `ox_persistence_ldap` | LDAP / LDAPS directory | Map canonical entities to directory entries, `memberOf`, and custom attributes or auxiliary object classes for grant storage |
| `ox_persistence_ad` | Active Directory | Extends LDAP driver with AD-specific schema; maps groups to AD security groups, grants to custom AD attributes or group policy extensions |
| `ox_persistence_okta` | Okta REST API | Map principals to Okta users, groups to Okta groups, grants to custom roles or profile attributes |
| `ox_persistence_radius` | RADIUS server | User and group membership only — no native grant storage; overflow to local store |
| `ox_persistence_tacacs` | TACACS+ server | Authentication and group membership; accounting records forwarded via TACACS+ accounting |

### 2. Schema Translation & Overflow

Backends vary in how much of the canonical IAM schema they can natively represent. The data layer must handle this transparently:

- **Driver capability declaration** — each driver declares which canonical fields it can natively store (e.g., an LDAP driver with a basic schema can store `PrincipalRecord` and `GroupMember` but not `PermissionGrant`)
- **Overflow storage** — fields that the primary backend cannot represent are automatically routed to a configured local overflow store (SQL) by the data layer, invisible to the security crates
- **Composite reads** — when hydrating an entity, the data layer assembles the full canonical object from the primary backend and any overflow store, returning a single complete result

This is an extension to the existing `DataObjectManager` / `QueryEngine` cross-datasource join capability. The overflow routing logic belongs in the dictionary/mapping layer, not in individual drivers.

### 3. Field-Level Encryption

Sensitive IAM fields (API keys, session tokens, password hashes, MFA secrets) must not transit the data layer in the clear. Required:

- An encrypted `ValueType` variant or an attribute-level encryption annotation in the data dictionary
- Encryption/decryption applied transparently by the persistence addon before serialization and after deserialization
- Key management (at minimum: a configured key reference; key rotation out of scope for initial delivery)

### 4. Cross-Datasource Atomic Transactions

The existing transaction spec describes ACID semantics but multi-store atomicity is not yet fully implemented. For IAM, a write that spans stores (e.g., `GroupMember` in LDAP + `PermissionGrant` in SQL + `AuditEvent` in SQL) must either fully commit or fully roll back.

Required: complete the multi-store transaction commit/rollback path in `ox_transaction` so that transaction boundaries work correctly when a single GDO save touches more than one datasource.

### 5. Audit Trail Addon

The security layer's `AuditEvent` entity requires tamper-evident append-only storage. Required:

- An `Auditable` addon (parallel to `Persistent`, `Transactable`) that automatically records who changed what and when on any GDO that opts in
- Audit records are append-only — no update or delete operations permitted on audit rows
- The addon fires via the existing callback mechanism (`after_save`, `after_delete`) so no GDO core changes are needed

### 6. Completed Cross-Datasource Query Execution

The `QueryEngine` / `QueryPlan` / `QueryNode` spec exists but execution across multiple datasources is incomplete. The security layer's evaluation algorithm requires reliably joining group membership (potentially from LDAP) with grant records (potentially from SQL) at check time. The cross-datasource join execution path in `ox_data_object_manager` must be completed.

### Summary

| Item | Blocking For |
|---|---|
| `ox_persistence_ldap` driver | `LdapAuthDriver`, `LdapAuthzDriver` |
| `ox_persistence_ad` driver | `AdAuthDriver`, `AdAuthzDriver` |
| `ox_persistence_okta` driver | `OktaAuthzDriver` |
| `ox_persistence_radius` driver | `RadiusAuthDriver` |
| `ox_persistence_tacacs` driver | `TacacsAuthDriver`, `TacacsAccountingDriver` |
| Schema translation & overflow routing | All non-SQL backends |
| Field-level encryption | Session tokens, API keys, MFA secrets |
| Multi-store atomic transactions | Any IAM write spanning two backends |
| Audit trail addon | `AuditEvent` storage, compliance logging |
| Cross-datasource query completion | Group + grant join at authz check time |