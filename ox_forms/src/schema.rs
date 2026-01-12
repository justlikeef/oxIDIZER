use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ModuleSchema {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub forms: Vec<FormDefinition>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FormDefinition {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub fields: Vec<FieldDefinition>,
    pub layout: Option<LayoutDefinition>,
    #[serde(default)]
    pub actions: Vec<ActionDefinition>,
    #[serde(default)]
    pub data_source_binding: Option<String>,
    #[serde(default)]
    pub style: Option<String>, // Legacy - keep for compat
    #[serde(default)]
    pub classes: Option<String>,
    #[serde(default)]
    pub styles: Option<String>,
    #[serde(default)]
    pub condition: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FieldDefinition {
    pub name: String,
    #[serde(default)]
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
    pub classes: Option<String>,
    #[serde(default)]
    pub styles: Option<String>,
    #[serde(default)]
    pub props: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_value: Option<Value>,
    #[serde(default)]
    pub condition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subfields: Option<Vec<FieldDefinition>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subforms: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ActionDefinition {
    pub name: String,
    pub label: String,
    pub action_type: String, // "submit", "reset", "button"
    pub component: Option<String>,
    #[serde(default)]
    pub props: Value,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ValidationRule {
    pub rule_type: String, // "required", "min", "max", "regex"
    #[serde(default)]
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
