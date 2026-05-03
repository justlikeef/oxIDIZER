use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use ox_type_converter::ValueType;
use ox_data_error::OxDataError;

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

    pub fn save_to_file<P: AsRef<std::path::Path>>(&self, path: P) -> Result<(), OxDataError> {
        let json = serde_json::to_string_pretty(self).map_err(|e| OxDataError::InternalError(e.to_string()))?;
        std::fs::write(path, json).map_err(|e| OxDataError::InternalError(e.to_string()))?;
        Ok(())
    }

    pub fn load_from_file<P: AsRef<std::path::Path>>(path: P) -> Result<Self, OxDataError> {
        let data = std::fs::read_to_string(path).map_err(|e| OxDataError::InternalError(e.to_string()))?;
        let dict = serde_json::from_str(&data).map_err(|e| OxDataError::InternalError(e.to_string()))?;
        Ok(dict)
    }

    /// Register a schema builder, converting it into the appropriate
    /// DataStoreContainer and DataObjectDefinition entries.
    pub fn register_schema(&mut self, schema: DataObjectSchema) -> Result<(), OxDataError> {
        let container_id = schema.name.clone();
        
        let fields: Vec<DataStoreField> = schema.fields.iter().map(|fd| {
            let mut parameters = HashMap::new();
            if fd.is_primary_key { parameters.insert("primary_key".to_string(), "true".to_string()); }
            if fd.is_indexed { parameters.insert("indexed".to_string(), "true".to_string()); }
            if fd.is_auto_increment { parameters.insert("auto_increment".to_string(), "true".to_string()); }
            DataStoreField {
                name: fd.name.clone(),
                data_type: fd.data_type.clone(),
                parameters,
                description: None,
            }
        }).collect();

        let container = DataStoreContainer {
            id: container_id.clone(),
            datasource_id: "default".to_string(),
            name: schema.name.clone(),
            container_type: "table".to_string(),
            fields,
            metadata: HashMap::new(),
        };

        let attributes: Vec<DataObjectAttribute> = schema.fields.iter().map(|fd| {
            DataObjectAttribute {
                name: fd.name.clone(),
                data_type: fd.data_type.clone(),
                mapping: AttributeMapping::Direct {
                    container_id: container_id.clone(),
                    field_name: fd.name.clone(),
                },
                description: None,
                validation: None,
            }
        }).collect();

        let object = DataObjectDefinition {
            id: schema.name.clone(),
            name: schema.name.clone(),
            description: None,
            attributes,
            relationships: vec![],
        };

        self.add_container(container);
        self.add_object(object);
        Ok(())
    }
}

/// A builder for defining a data object schema (table-like structure).
/// Used by subsystems (e.g. ox_cert) to declaratively register their schemas
/// with the DataDictionary.
#[derive(Debug, Clone)]
pub struct DataObjectSchema {
    pub name: String,
    pub fields: Vec<FieldDescriptor>,
}

impl DataObjectSchema {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            fields: Vec::new(),
        }
    }

    pub fn add_field(&mut self, field: FieldDescriptor) {
        self.fields.push(field);
    }
}

/// Describes a single field/column in a DataObjectSchema.
/// Supports builder-pattern methods for common constraints.
#[derive(Debug, Clone)]
pub struct FieldDescriptor {
    pub name: String,
    pub data_type: ValueType,
    pub is_primary_key: bool,
    pub is_indexed: bool,
    pub is_auto_increment: bool,
}

impl FieldDescriptor {
    pub fn new(name: &str, data_type: ValueType) -> Self {
        Self {
            name: name.to_string(),
            data_type,
            is_primary_key: false,
            is_indexed: false,
            is_auto_increment: false,
        }
    }

    pub fn primary_key(mut self) -> Self {
        self.is_primary_key = true;
        self
    }

    pub fn indexed(mut self) -> Self {
        self.is_indexed = true;
        self
    }

    pub fn auto_increment(mut self) -> Self {
        self.is_auto_increment = true;
        self
    }
}
