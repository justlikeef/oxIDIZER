#[cfg(test)]
mod tests {
    use crate::dictionary::*;
    use crate::query::*;
    use ox_type_converter::ValueType;
    use ox_persistence::{PersistenceDriver, register_persistence_driver, DriverMetadata, DataSet};
    use std::collections::HashMap;
    use std::sync::Arc;

    struct MockDriver {
        data: HashMap<String, HashMap<String, (String, ValueType, HashMap<String, String>)>>,
    }

    impl PersistenceDriver for MockDriver {
        fn persist(&self, _data: &HashMap<String, (String, ValueType, HashMap<String, String>)>, _location: &str) -> Result<(), String> {
            Ok(())
        }
        fn restore(&self, _location: &str, id: &str) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, String> {
            self.data.get(id).cloned().ok_or("Not found".to_string())
        }
        fn fetch(&self, _filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>, _location: &str) -> Result<Vec<String>, String> {
            Ok(self.data.keys().cloned().collect())
        }
        fn notify_lock_status_change(&self, _status: &str, _id: &str) {}
        fn prepare_datastore(&self, _info: &HashMap<String, String>) -> Result<(), String> { Ok(()) }
        fn list_datasets(&self, _info: &HashMap<String, String>) -> Result<Vec<String>, String> { Ok(vec![]) }
        fn describe_dataset(&self, _info: &HashMap<String, String>, _name: &str) -> Result<DataSet, String> { Err("Not implemented".to_string()) }
        fn get_connection_parameters(&self) -> Vec<ox_persistence::ConnectionParameter> { vec![] }
    }

    #[test]
    fn test_dictionary_composition() {
        let mut dict = DataDictionary::new();

        // 1. Define a physical container (e.g. MySQL Table)
        let mysql_table = DataStoreContainer {
            id: "mysql:users".to_string(),
            datasource_id: "mysql_prod".to_string(),
            name: "users".to_string(),
            container_type: "table".to_string(),
            fields: vec![
                DataStoreField {
                    name: "id".to_string(),
                    data_type: ValueType::Integer,
                    parameters: HashMap::new(),
                    description: Some("Primary Key".to_string()),
                },
                DataStoreField {
                    name: "username".to_string(),
                    data_type: ValueType::String,
                    parameters: HashMap::new(),
                    description: None,
                },
            ],
            metadata: HashMap::new(),
        };
        dict.add_container(mysql_table);

        // 2. Define a logical object (GDO) mapping
        let user_object = DataObjectDefinition {
            id: "user_logical".to_string(),
            name: "User".to_string(),
            description: Some("Logical User Object".to_string()),
            attributes: vec![
                DataObjectAttribute {
                    name: "external_id".to_string(),
                    data_type: ValueType::Integer,
                    mapping: AttributeMapping::Direct {
                        container_id: "mysql:users".to_string(),
                        field_name: "id".to_string(),
                    },
                    description: None,
                    validation: None,
                },
                DataObjectAttribute {
                    name: "login".to_string(),
                    data_type: ValueType::String,
                    mapping: AttributeMapping::Direct {
                        container_id: "mysql:users".to_string(),
                        field_name: "username".to_string(),
                    },
                    description: None,
                    validation: None,
                },
            ],
            relationships: vec![],
        };
        dict.add_object(user_object);

        assert_eq!(dict.containers.len(), 1);
        assert_eq!(dict.objects.len(), 1);
        
        let obj = dict.objects.get("user_logical").unwrap();
        assert_eq!(obj.attributes.len(), 2);

        // 3. Test Persistence
        let temp_file = "test_dict.json";
        dict.save_to_file(temp_file).unwrap();
        let loaded_dict = DataDictionary::load_from_file(temp_file).unwrap();
        assert_eq!(loaded_dict.containers.len(), 1);
        assert_eq!(loaded_dict.objects.len(), 1);
        let _ = std::fs::remove_file(temp_file);
    }

    #[test]
    fn test_query_engine_emulated_join() {
        // 1. Setup Mock Data
        let mut user_data = HashMap::new();
        let mut row1 = HashMap::new();
        row1.insert("id".to_string(), ("1".to_string(), ValueType::Integer, HashMap::new()));
        row1.insert("username".to_string(), ("alice".to_string(), ValueType::String, HashMap::new()));
        user_data.insert("1".to_string(), row1);

        let mut profile_data = HashMap::new();
        let mut row2 = HashMap::new();
        row2.insert("user_id".to_string(), ("1".to_string(), ValueType::Integer, HashMap::new()));
        row2.insert("bio".to_string(), ("Hello world".to_string(), ValueType::String, HashMap::new()));
        profile_data.insert("p1".to_string(), row2);

        // 2. Register Mock Drivers
        let driver1 = Arc::new(MockDriver { data: user_data });
        let driver2 = Arc::new(MockDriver { data: profile_data });

        let meta1 = DriverMetadata { name: "mock_users".to_string(), version: "0.1.0".to_string(), description: "".to_string(), compatible_modules: HashMap::new() };
        let meta2 = DriverMetadata { name: "mock_profiles".to_string(), version: "0.1.0".to_string(), description: "".to_string(), compatible_modules: HashMap::new() };

        register_persistence_driver(driver1, meta1);
        register_persistence_driver(driver2, meta2);

        // 3. Define Query Plan (Join mock_users and mock_profiles)
        let plan = QueryPlan {
            root: QueryNode::Join {
                left: Box::new(QueryNode::Fetch {
                    container_id: "c1".to_string(),
                    datasource_id: "mock_users".to_string(),
                    location: "users".to_string(),
                    filters: HashMap::new(),
                }),
                right: Box::new(QueryNode::Fetch {
                    container_id: "c2".to_string(),
                    datasource_id: "mock_profiles".to_string(),
                    location: "profiles".to_string(),
                    filters: HashMap::new(),
                }),
                join_type: JoinType::Inner,
                conditions: vec![JoinCondition {
                    from_field: "id".to_string(),
                    to_field: "user_id".to_string(),
                    operator: "=".to_string(),
                }],
            }
        };

        // 4. Execute
        let engine = QueryEngine::new();
        let results = engine.execute_plan(&plan).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].get("username").unwrap().0, "alice");
        assert_eq!(results[0].get("bio").unwrap().0, "Hello world");
    }

    #[test]
    fn test_relationships() {
        let rel = RelationshipDefinition {
            id: "user_to_profile".to_string(),
            from_container_id: "mysql:users".to_string(),
            to_container_id: "json:profiles".to_string(),
            join_type: JoinType::Inner,
            conditions: vec![JoinCondition {
                from_field: "id".to_string(),
                to_field: "user_id".to_string(),
                operator: "=".to_string(),
            }],
        };
        
        assert_eq!(rel.join_type, JoinType::Inner);
        assert_eq!(rel.conditions[0].from_field, "id");
    }

    #[test]
    fn test_load_data_object_flow() {
        use crate::DataObjectManager;
        
        // 1. Setup Data Dictionary
        let mut dict = DataDictionary::new();
        // Container
        let container = DataStoreContainer {
            id: "mysql:users".to_string(),
            datasource_id: "mock_load_source".to_string(), // Unique name
            name: "users".to_string(),
            container_type: "table".to_string(),
            fields: vec![],
            metadata: HashMap::new(),
        };
        dict.add_container(container);
        
        // Object Definition
        let user_def = DataObjectDefinition {
            id: "User".to_string(),
            name: "User".to_string(),
            description: None,
            attributes: vec![
                DataObjectAttribute {
                    name: "id".to_string(),
                    data_type: ValueType::Integer,
                    mapping: AttributeMapping::Direct {
                         container_id: "mysql:users".to_string(),
                         field_name: "db_id".to_string(),
                    },
                    description: None,
                    validation: None,
                },
                DataObjectAttribute {
                     name: "email".to_string(),
                     data_type: ValueType::String,
                     mapping: AttributeMapping::Direct {
                          container_id: "mysql:users".to_string(),
                          field_name: "db_email".to_string(),
                     },
                     description: None,
                     validation: None,
                },
            ],
            relationships: vec![],
        };
        dict.add_object(user_def);
        
        // 2. Register Mock Driver
        let mut mock_data = HashMap::new();
        let mut row = HashMap::new();
        row.insert("db_id".to_string(), ("100".to_string(), ValueType::Integer, HashMap::new()));
        row.insert("db_email".to_string(), ("test@example.com".to_string(), ValueType::String, HashMap::new()));
        mock_data.insert("100".to_string(), row);
        
        let driver = Arc::new(MockDriver { data: mock_data });
        let meta = DriverMetadata { name: "mock_load_source".to_string(), version: "1".to_string(), description: "".to_string(), compatible_modules: HashMap::new() };
        register_persistence_driver(driver, meta);
        
        // 3. Manager
        let manager = DataObjectManager::with_dictionary(dict);
        
        // 4. Load
        let gdo = manager.load_data_object("User", "100").expect("Failed to load");
        
        // 5. Verify
        assert_eq!(gdo.get::<String>("email").unwrap(), "test@example.com");
        assert_eq!(gdo.get::<i64>("id").unwrap(), 100);
    }
}
