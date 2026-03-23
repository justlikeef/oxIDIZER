use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use ox_type_converter::ValueType;

/// Physical representation of a data storage unit (table, view, file, etc.)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DataStoreContainer {
    pub id: String,
    pub datasource_id: String,
    pub name: String,         // e.g. "users" table name
    pub container_type: String, // "table", "view", "file", "key"
    pub fields: Vec<DataStoreField>,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DataStoreField {
    pub name: String,
    pub data_type: ValueType,
    pub parameters: HashMap<String, String>,
    pub description: Option<String>,
}

/// Logical representation of a Data Object
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DataObjectDefinition {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub attributes: Vec<DataObjectAttribute>,
    pub relationships: Vec<RelationshipDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DataObjectAttribute {
    pub name: String,
    pub data_type: ValueType,
    pub mapping: AttributeMapping,
    pub description: Option<String>,
    #[serde(default)]
    pub validation: Option<Vec<AttributeValidation>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttributeValidation {
    pub rule_type: String, // "required", "min", "max", "regex"
    pub parameters: HashMap<String, String>, // Simple KV params for now
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum AttributeMapping {
    Direct {
        container_id: String,
        field_name: String,
    },
    Calculated {
        expression: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelationshipDefinition {
    pub id: String,
    pub from_container_id: String,
    pub to_container_id: String,
    pub join_type: JoinType,
    pub conditions: Vec<JoinCondition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum JoinType {
    Inner,
    Left,
    Right,
    Outer,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JoinCondition {
    pub from_field: String,
    pub to_field: String,
    pub operator: String, // e.g. "="
}

/// The global dictionary containing all metadata
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DataDictionary {
    pub containers: HashMap<String, DataStoreContainer>,
    pub objects: HashMap<String, DataObjectDefinition>,
}

impl DataDictionary {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_container(&mut self, container: DataStoreContainer) {
        self.containers.insert(container.id.clone(), container);
    }

    pub fn add_object(&mut self, object: DataObjectDefinition) {
        self.objects.insert(object.id.clone(), object);
    }

    /// Merges a new container definition into the dictionary.
    /// If the container already exists, it updates the fields but preserves existing identity.
    pub fn merge_container(&mut self, new_container: DataStoreContainer) {
        if let Some(existing) = self.containers.get_mut(&new_container.id) {
            // Update fields but we could keep some metadata if we wanted.
            // For now, we sync the fields.
            existing.fields = new_container.fields;
            existing.name = new_container.name;
        } else {
            self.containers.insert(new_container.id.clone(), new_container);
        }
    }

    pub fn save_to_file<P: AsRef<std::path::Path>>(&self, path: P) -> anyhow::Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load_from_file<P: AsRef<std::path::Path>>(path: P) -> anyhow::Result<Self> {
        let data = std::fs::read_to_string(path)?;
        let dict = serde_json::from_str(&data)?;
        Ok(dict)
    }
}
