use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FormDefinition {
    pub id: String,
    pub title: String,
    pub fields: Vec<FieldDefinition>,
    pub layout: Option<LayoutDefinition>,
    pub actions: Vec<ActionDefinition>,
    pub data_source_binding: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FieldDefinition {
    pub name: String,
    pub label: String,
    /// Abstract data type (e.g., "email", "currency")
    pub data_type: String,
    /// Concrete component override (e.g., "my-custom-email-input")
    pub component: Option<String>,
    /// Explicit plugins/hooks to specific field instance
    #[serde(default)]
    pub plugins: Vec<String>,
    #[serde(default)]
    pub validation: Vec<ValidationRule>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub props: Value,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ActionDefinition {
    pub name: String,
    pub label: String,
    pub action_type: String, // "submit", "reset", "button"
    #[serde(default)]
    pub props: Value,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ValidationRule {
    pub rule_type: String, // "required", "min", "max", "regex"
    pub parameters: Value,
    pub message: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LayoutDefinition {
    pub items: Vec<LayoutItem>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum LayoutItem {
    Row { 
        items: Vec<LayoutItem>, 
        classes: Option<String> 
    },
    Column { 
        items: Vec<LayoutItem>, 
        width: Option<u8> // 1-12 usually
    },
    Field { 
        name: String 
    },
    HTML { 
        content: String 
    },
    Tabs { 
        tabs: Vec<TabDefinition> 
    },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TabDefinition {
    pub label: String,
    pub content: Vec<LayoutItem>,
}
