//! A small embedded database with B-tree storage and rollback journaling.
//!
//! ```
//! use oxidedb::{Column, DataType, Database, Row, Schema, Value};
//!
//! let path = std::env::temp_dir().join(format!("oxidedb-example-{}", std::process::id()));
//! let mut db = Database::create(&path)?;
//! db.create_table(Schema::new(
//!     "users".into(),
//!     vec![Column::new("id".into(), DataType::Integer, false, true)],
//! )?)?;
//! db.insert("users", Row::new(vec![Value::Integer(1)]))?;
//! assert_eq!(db.get("users", &Value::Integer(1))?, Some(Row::new(vec![Value::Integer(1)])));
//! drop(db);
//! std::fs::remove_dir_all(path)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
mod db;
mod storage;

pub use db::{Column, DataType, Database, DbError, Result, Row, Schema, Value};
