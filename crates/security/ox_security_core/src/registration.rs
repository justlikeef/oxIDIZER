use crate::operations::OperationDef;

#[derive(Debug, Clone)]
pub struct ContextDefinition {
    pub root: &'static str,
    pub operations: &'static [OperationDef],
    pub children: &'static [ContextDefinition],
}

impl ContextDefinition {
    /// Returns the union of all operations in this node and its entire subtree.
    /// This is the set of operations that can be granted at this node.
    pub fn all_operations(&self) -> Vec<OperationDef> {
        let mut ops: Vec<OperationDef> = self.operations.to_vec();
        for child in self.children {
            for op in child.all_operations() {
                if !ops.iter().any(|o| o.name == op.name) {
                    ops.push(op);
                }
            }
        }
        ops
    }
}

/// Implemented by objects that participate in the permission model.
/// Objects describe only their own fragment — they have no knowledge of
/// which application or call context uses them.
pub trait SecurityRegistration {
    fn context_definition(&self) -> ContextDefinition;
}

/// Implemented by SecurityPipeline. Consuming crates call this at startup
/// to register their context tree fragment.
pub trait ContextRegistrar {
    fn register_context(&self, def: ContextDefinition);
}
