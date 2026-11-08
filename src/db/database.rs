use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use super::error::{DbError, Result};
use super::row::Row;
use super::schema::Schema;
use super::table::Table;
use super::value::Value;

pub struct Database {
    directory: PathBuf,
    tables: HashMap<String, Table>,
    _lock: File,
}

impl Database {
    pub fn create(path: &Path) -> Result<Self> {
        fs::create_dir(path)?;
        Self::open(path)
    }

    pub fn open(path: &Path) -> Result<Self> {
        let directory = fs::canonicalize(path)?;
        if !directory.is_dir() {
            return Err(std::io::Error::new(
                ErrorKind::NotADirectory,
                "database path is not a directory",
            )
            .into());
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join(".oxidedb.lock"))?;
        lock.try_lock().map_err(std::io::Error::from)?;
        Ok(Self {
            directory,
            tables: HashMap::new(),
            _lock: lock,
        })
    }

    pub fn create_table(&mut self, schema: Schema) -> Result<()> {
        let name = schema.table_name().to_owned();
        let path = self.table_path(&name)?;
        if self.tables.contains_key(&name) {
            return Err(DbError::TableAlreadyExists(name));
        }
        let table = Table::create(&path, schema).map_err(|error| table_error(error, &name))?;
        self.tables.insert(name, table);
        Ok(())
    }

    fn table(&mut self, name: &str) -> Result<&mut Table> {
        if !self.tables.contains_key(name) {
            let path = self.table_path(name)?;
            let metadata =
                fs::symlink_metadata(&path).map_err(|error| table_error(error.into(), name))?;
            if !metadata.is_file() {
                return Err(DbError::CorruptData(format!(
                    "table '{name}' is not a regular file"
                )));
            }
            let table = Table::open(&path).map_err(|error| table_error(error, name))?;
            if table.schema().table_name() != name {
                return Err(DbError::CorruptData(format!(
                    "schema name does not match table '{name}'"
                )));
            }
            self.tables.insert(name.to_owned(), table);
        }
        Ok(self.tables.get_mut(name).expect("opened table must be cached"))
    }

    pub fn insert(&mut self, table: &str, row: Row) -> Result<()> {
        self.table(table)?.insert(row)
    }

    pub fn get(&mut self, table: &str, key: &Value) -> Result<Option<Row>> {
        self.table(table)?.get(key)
    }

    pub fn replace(&mut self, table: &str, key: &Value, row: Row) -> Result<bool> {
        self.table(table)?.replace(key, row)
    }

    pub fn delete(&mut self, table: &str, key: &Value) -> Result<bool> {
        self.table(table)?.delete(key)
    }

    pub fn scan(&mut self, table: &str) -> Result<Vec<Row>> {
        self.table(table)?.scan()
    }

    pub fn table_names(&self) -> Result<Vec<String>> {
        let mut names = Vec::new();
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let file_name = entry.file_name();
            let Some(file_name) = file_name.to_str() else {
                continue;
            };
            let Some(encoded) = file_name.strip_prefix("table-").and_then(|name| name.strip_suffix(".db"))
            else {
                continue;
            };
            if encoded.is_empty()|| encoded.len() % 2 != 0|| !encoded.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err(DbError::CorruptData(format!("invalid table filename '{file_name}'")));
            }
            let bytes = (0..encoded.len())
                .step_by(2)
                .map(|index| {
                    u8::from_str_radix(&encoded[index..index + 2], 16)
                        .expect("validated hex filename")
                })
                .collect();
            let name = String::from_utf8(bytes).map_err(|_| {
                DbError::CorruptData(format!("invalid table filename '{file_name}'"))
            })?;
            if name.trim().is_empty() {
                return Err(DbError::CorruptData("empty table name in filename".into()));
            }
            names.push(name);
        }
        names.sort();
        Ok(names)
    }

    pub fn drop_table(&mut self, name: &str) -> Result<()> {
        let path = self.table_path(name)?;
        self.table(name)?;
        fs::remove_file(path).map_err(|error| table_error(error.into(), name))?;
        self.tables.remove(name);
        Ok(())
    }

    fn table_path(&self, name: &str) -> Result<PathBuf> {
        if name.trim().is_empty() {
            return Err(DbError::InvalidSchema("table name cannot be empty".into()));
        }
        let encoded: String = name.bytes().map(|byte| format!("{byte:02x}")).collect();
        Ok(self.directory.join(format!("table-{encoded}.db")))
    }
}

fn table_error(error: DbError, name: &str) -> DbError {
    match error {
        DbError::Io(error) if error.kind() == ErrorKind::NotFound => {
            DbError::TableNotFound(name.to_owned())
        }
        DbError::Io(error) if error.kind() == ErrorKind::AlreadyExists => {
            DbError::TableAlreadyExists(name.to_owned())
        }
        error => error,
    }
}
