#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationDef {
    pub name: &'static str,
    pub description: &'static str,
}

pub const OP_READ:    OperationDef = OperationDef { name: "read",    description: "Read a value or record" };
pub const OP_WRITE:   OperationDef = OperationDef { name: "write",   description: "Write a value or record" };
pub const OP_CREATE:  OperationDef = OperationDef { name: "create",  description: "Create a new record" };
pub const OP_CHANGE:  OperationDef = OperationDef { name: "change",  description: "Modify an existing record" };
pub const OP_DELETE:  OperationDef = OperationDef { name: "delete",  description: "Delete a record" };
pub const OP_LIST:    OperationDef = OperationDef { name: "list",    description: "List or enumerate records" };
pub const OP_EXECUTE: OperationDef = OperationDef { name: "execute", description: "Execute a function or procedure" };
pub const OP_DDL:     OperationDef = OperationDef { name: "ddl",     description: "Modify schema or structure" };
