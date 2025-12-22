
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SqlDialect {
    Mysql,
    Postgres,
    Mssql,
    Sqlite,
}

pub struct SqlBuilder {
    dialect: SqlDialect,
}

impl SqlBuilder {
    pub fn new(dialect: SqlDialect) -> Self {
        SqlBuilder { dialect }
    }

    fn quote_identifier(&self, identifier: &str) -> String {
        match self.dialect {
            SqlDialect::Mysql => format!("`{}`", identifier),
            SqlDialect::Postgres | SqlDialect::Sqlite => format!("\"{}\"", identifier),
            SqlDialect::Mssql => format!("[{}]", identifier),
        }
    }

    fn placeholder(&self, index: usize) -> String {
        match self.dialect {
            SqlDialect::Mysql | SqlDialect::Sqlite => "?".to_string(),
            SqlDialect::Postgres => format!("${}", index + 1),
            SqlDialect::Mssql => format!("@P{}", index + 1),
        }
    }

    pub fn build_insert(&self, table: &str, keys: &[String]) -> String {
        let quoted_table = self.quote_identifier(table);
        let quoted_keys: Vec<String> = keys.iter().map(|k| self.quote_identifier(k)).collect();
        let cols = quoted_keys.join(", ");
        
        let placeholders: Vec<String> = (0..keys.len())
            .map(|i| self.placeholder(i))
            .collect();
        let vals = placeholders.join(", ");

        format!("INSERT INTO {} ({}) VALUES ({})", quoted_table, cols, vals)
    }

    pub fn build_select_by_id(&self, table: &str) -> String {
        let quoted_table = self.quote_identifier(table);
        let placeholder = self.placeholder(0);
        
        // Assuming ID column assumes specific quoting? Usually "id" or [id] or `id`.
        // Let's assume standard "id" column name, quoted.
        let quoted_id = self.quote_identifier("id");

        format!("SELECT * FROM {} WHERE {} = {}", quoted_table, quoted_id, placeholder)
    }

    pub fn build_fetch(&self, table: &str, keys: &[String]) -> String {
        let quoted_table = self.quote_identifier(table);
        let quoted_id = self.quote_identifier("id");

        let mut query = format!("SELECT {} FROM {} WHERE 1=1", quoted_id, quoted_table);
        
        for (i, key) in keys.iter().enumerate() {
            let quoted_key = self.quote_identifier(key);
            let placeholder = self.placeholder(i);
            query.push_str(&format!(" AND {} = {}", quoted_key, placeholder));
        }

        query
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mysql_insert() {
        let builder = SqlBuilder::new(SqlDialect::Mysql);
        let keys = vec!["name".to_string(), "age".to_string()];
        let sql = builder.build_insert("users", &keys);
        assert_eq!(sql, "INSERT INTO `users` (`name`, `age`) VALUES (?, ?)");
    }

    #[test]
    fn test_postgres_insert() {
        let builder = SqlBuilder::new(SqlDialect::Postgres);
        let keys = vec!["name".to_string(), "age".to_string()];
        let sql = builder.build_insert("users", &keys);
        assert_eq!(sql, "INSERT INTO \"users\" (\"name\", \"age\") VALUES ($1, $2)");
    }

    #[test]
    fn test_mssql_insert() {
        let builder = SqlBuilder::new(SqlDialect::Mssql);
        let keys = vec!["name".to_string(), "age".to_string()];
        let sql = builder.build_insert("users", &keys);
        assert_eq!(sql, "INSERT INTO [users] ([name], [age]) VALUES (@P1, @P2)");
    }
}
