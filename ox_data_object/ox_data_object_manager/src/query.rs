use serde::{Deserialize, Serialize};
use crate::dictionary::{JoinType, JoinCondition};
use std::collections::HashMap;
use ox_persistence::PERSISTENCE_DRIVER_REGISTRY;
use anyhow::Result;
use ox_type_converter::ValueType;

/// A node in the execution plan for a data query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum QueryNode {
    /// Fetch data from a specific physical container.
    Fetch {
        container_id: String,
        datasource_id: String,
        location: String,
        filters: HashMap<String, String>,
    },
    /// Perform a join between two query results.
    Join {
        left: Box<QueryNode>,
        right: Box<QueryNode>,
        join_type: JoinType,
        conditions: Vec<JoinCondition>,
    },
}

/// A serialized plan for executing a cross-datasource query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryPlan {
    pub root: QueryNode,
}

pub struct QueryEngine;

impl QueryEngine {
    pub fn new() -> Self {
        Self
    }

    /// Executes a query plan and returns the result as a list of flat maps (rows).
    pub fn execute_plan(&self, plan: &QueryPlan) -> Result<Vec<HashMap<String, (String, ValueType, HashMap<String, String>)>>> {
        self.execute_node(&plan.root)
    }

    fn execute_node(&self, node: &QueryNode) -> Result<Vec<HashMap<String, (String, ValueType, HashMap<String, String>)>>> {
        match node {
            QueryNode::Fetch { datasource_id, location, .. } => {
                let registry = PERSISTENCE_DRIVER_REGISTRY.lock().unwrap();
                if let Some((driver, _)) = registry.get_driver(datasource_id) {
                    // Fetch all IDs first.
                    // This is a simplified fetch. In reality, we'd pass filters.
                    let ids = driver.fetch(&HashMap::new(), location).map_err(|e| anyhow::anyhow!(e))?;
                    let mut results = Vec::new();
                    for id in ids {
                        let map = driver.restore(location, &id).map_err(|e| anyhow::anyhow!(e))?;
                        results.push(map);
                    }
                    Ok(results)
                } else {
                    Err(anyhow::anyhow!("Driver {} not found", datasource_id))
                }
            }
            QueryNode::Join { left, right, join_type, conditions } => {
                let left_results = self.execute_node(left)?;
                let right_results = self.execute_node(right)?;
                self.perform_join(left_results, right_results, join_type, conditions)
            }
        }
    }

    fn perform_join(
        &self,
        left: Vec<HashMap<String, (String, ValueType, HashMap<String, String>)>>,
        right: Vec<HashMap<String, (String, ValueType, HashMap<String, String>)>>,
        join_type: &JoinType,
        conditions: &[JoinCondition],
    ) -> Result<Vec<HashMap<String, (String, ValueType, HashMap<String, String>)>>> {
        let mut results = Vec::new();

        match join_type {
            JoinType::Inner => {
                for l_row in &left {
                    for r_row in &right {
                        if self.matches_conditions(l_row, r_row, conditions) {
                            results.push(self.merge_rows(l_row, r_row));
                        }
                    }
                }
            }
            JoinType::Left => {
                for l_row in &left {
                    let mut matched = false;
                    for r_row in &right {
                        if self.matches_conditions(l_row, r_row, conditions) {
                            results.push(self.merge_rows(l_row, r_row));
                            matched = true;
                        }
                    }
                    if !matched {
                        results.push(l_row.clone());
                    }
                }
            }
            _ => return Err(anyhow::anyhow!("Join type {:?} not yet implemented", join_type)),
        }

        Ok(results)
    }

    fn matches_conditions(
        &self, 
        left: &HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        right: &HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        conditions: &[JoinCondition]
    ) -> bool {
        for cond in conditions {
            let left_val = left.get(&cond.from_field).map(|v| &v.0);
            let right_val = right.get(&cond.to_field).map(|v| &v.0);
            
            if left_val.is_none() || right_val.is_none() || left_val != right_val {
                return false;
            }
        }
        true
    }

    fn merge_rows(
        &self, 
        left: &HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        right: &HashMap<String, (String, ValueType, HashMap<String, String>)>
    ) -> HashMap<String, (String, ValueType, HashMap<String, String>)> {
        let mut merged = left.clone();
        for (k, v) in right {
            merged.insert(k.clone(), v.clone());
        }
        merged
    }
}
