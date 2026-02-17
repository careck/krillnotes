use crate::Result;
use rusqlite::Connection;
use std::path::Path;

pub struct Storage {
    conn: Connection,
}

impl Storage {
    pub fn create<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(include_str!("schema.sql"))?;
        Ok(Self { conn })
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_create_storage() {
        let temp = NamedTempFile::new().unwrap();
        let storage = Storage::create(temp.path()).unwrap();

        // Verify tables exist
        let tables: Vec<String> = storage
            .connection()
            .prepare("SELECT name FROM sqlite_master WHERE type='table'")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();

        assert!(tables.contains(&"notes".to_string()));
        assert!(tables.contains(&"operations".to_string()));
        assert!(tables.contains(&"workspace_meta".to_string()));
    }
}
