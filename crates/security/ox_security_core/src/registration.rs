use crate::operations::OperationDef;

#[derive(Debug, Clone, Copy)]
pub struct ContextDefinition {
    pub root: &'static str,
    pub operations: &'static [OperationDef],
    pub children: &'static [ContextDefinition],
}

impl ContextDefinition {
    /// This is the set of operations that can be granted at this node.
    #[must_use]
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
/// Implementations must use interior mutability (`Arc<Mutex<...>>`) — `&self` is required
/// for object-safe use across threads.
pub trait ContextRegistrar {
    fn register_context(&self, def: ContextDefinition);
}
