# OxideDB

[![Crates.io](https://img.shields.io/crates/v/oxidedb.svg)](https://crates.io/crates/oxidedb)
[![Docs.rs](https://docs.rs/oxidedb/badge.svg)](https://docs.rs/oxidedb)
[![License](https://img.shields.io/crates/l/oxidedb.svg)](#7-license)

A small embedded database written in pure Rust, with B+ tree storage and crash-safe rollback journaling.

---

## Table of Contents

1. [What is OxideDB?](#1-what-is-oxidedb)
2. [Requirements](#2-requirements)
3. [Setup](#3-setup)
4. [API](#4-api)
5. [Examples](#5-examples)
6. [Limitations](#6-limitations)
7. [License](#7-license)

---

## 1. What is OxideDB?

OxideDB is a lightweight, embedded database that runs inside your own Rust program. There is no server and no network. You open a directory, define typed tables, and read and write rows through a plain Rust API.

- **B+ tree storage**: each table is a B+ tree of 4 KB pages, looked up by primary key.
- **Crash-safe writes**: every write is protected by a checksummed rollback journal, so a crash mid-write rolls back to the last committed state instead of leaving a corrupted table.
- **Typed schemas**: columns are typed (`Integer`, `Text`, `Float`, `Boolean`) with `NOT NULL` and primary key constraints.
- **Safe by default**: an exclusive lock prevents two handles from opening the same database at once.

## 2. Requirements

| Requirement | Version |
|---|---|
| Rust toolchain | **1.89 or newer** (edition 2024) |
| Cargo | Comes with Rust |
| Operating system | Any platform supported by Rust's standard library with file locking (Linux, macOS, Windows) |

The only dependency is [`atomicwrites`](https://crates.io/crates/atomicwrites), used for atomic file moves and replacements. Cargo downloads it automatically.

To check or update your toolchain:

```bash
rustc --version
rustup update stable
```

## 3. Setup

### Add it to your project

```bash
cargo add oxidedb
```

or add it to your `Cargo.toml` by hand:

```toml
[dependencies]
oxidedb = "0.1"
```

### Use the latest development version

```toml
[dependencies]
oxidedb = { git = "https://github.com/Mzt00/OxideDB" }
```

### Build from source

```bash
git clone https://github.com/Mzt00/OxideDB
cd OxideDB
cargo build --release
cargo test
```

### Import what you need

Everything is exported from the crate root:

```rust
use oxidedb::{Column, DataType, Database, DbError, Result, Row, Schema, Value};
```

## 4. API

### `Database`

A database is a **directory** on disk. Each table is stored in its own file inside it.

| Method | Returns | Description |
|---|---|---|
| `Database::create(path: &Path)` | `Result<Database>` | Creates a new database directory. Fails if the path already exists. |
| `Database::open(path: &Path)` | `Result<Database>` | Opens an existing database directory and takes an exclusive lock on it. |
| `create_table(schema: Schema)` | `Result<()>` | Creates a table from a schema. |
| `drop_table(name: &str)` | `Result<()>` | Deletes a table and its file. |
| `table_names()` | `Result<Vec<String>>` | Lists all tables, sorted alphabetically. |
| `insert(table: &str, row: Row)` | `Result<()>` | Inserts a row. Fails with `DuplicateKey` if the primary key already exists. |
| `get(table: &str, key: &Value)` | `Result<Option<Row>>` | Looks up a row by primary key. |
| `replace(table: &str, key: &Value, row: Row)` | `Result<bool>` | Replaces the row with the given key. Returns `false` if no such row exists. The primary key cannot change. |
| `delete(table: &str, key: &Value)` | `Result<bool>` | Deletes a row by primary key. Returns `false` if no such row exists. |
| `scan(table: &str)` | `Result<Vec<Row>>` | Returns every row in the table, ordered by primary key. |

### `Schema` and `Column`

```rust
Column::new(name: String, data_type: DataType, nullable: bool, primary_key: bool) -> Column

Schema::new(table_name: String, columns: Vec<Column>) -> Result<Schema>
```

`Schema::new` validates that:

- the table name and all column names are non-empty
- there is at least one column and no duplicate column names
- there is **exactly one** primary key column
- the primary key is `Integer` or `Text` and is **not** nullable

Useful accessors: `table_name()`, `columns()`, `column(name)`, `column_index(name)`, `primary_key_column()`, `primary_key_index()`.

### `Row`

| Method | Description |
|---|---|
| `Row::new(values: Vec<Value>)` | Creates a row. Values must be in column order. |
| `values()` | Returns all values as `&[Value]`. |
| `get(index)` | Returns `Option<&Value>` for a column index. |
| `set(index, value)` | Changes a value in place. Returns an error if the index is out of range. |

### `Value` and `DataType`

| `DataType` | `Value` variant | Rust type |
|---|---|---|
| `Integer` | `Value::Integer(i64)` | `i64` |
| `Text` | `Value::Text(String)` | `String` |
| `Float` | `Value::Float(f64)` | `f64` |
| `Boolean` | `Value::Boolean(bool)` | `bool` |
| n/a | `Value::Null` | allowed only in nullable columns |

`Value` also provides `is_null()`, `data_type()`, and implements `Display`.

### Errors

All fallible methods return `oxidedb::Result<T>`, which uses `DbError`. `DbError` implements `std::error::Error` and `Display`.

| Variant | Meaning |
|---|---|
| `Io` | Underlying file system error, including failing to acquire the lock. |
| `TableNotFound` / `TableAlreadyExists` | The table is missing, or already exists. |
| `ColumnNotFound` / `DuplicateColumn` | Schema column problems. |
| `InvalidSchema` / `InvalidRow` | The schema or row is malformed (wrong column count, bad key, and so on). |
| `TypeMismatch` | A value does not match its column's type. |
| `NullConstraintViolation` | `Null` was written to a non-nullable column. |
| `DuplicateKey` | A row with this primary key already exists. |
| `RowTooLarge` | The encoded row does not fit in one page. |
| `CorruptData` / `PageError` / `InvalidPageId` | The on-disk data failed validation. |
| `RecoveryRequired` / `CommitUnknown` | A write failed in a way that requires reopening the table. |

## 5. Examples

### Quick start

```rust
use oxidedb::{Column, DataType, Database, Row, Schema, Value};
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a new database (a directory on disk)
    let mut db = Database::create(Path::new("./my_db"))?;

    // Define a table
    let schema = Schema::new(
        "users".into(),
        vec![
            Column::new("id".into(), DataType::Integer, false, true),   // primary key
            Column::new("name".into(), DataType::Text, false, false),   // NOT NULL
            Column::new("age".into(), DataType::Integer, true, false),  // nullable
        ],
    )?;
    db.create_table(schema)?;

    // Insert rows
    db.insert("users", Row::new(vec![
        Value::Integer(1),
        Value::Text("Alice".into()),
        Value::Integer(30),
    ]))?;
    db.insert("users", Row::new(vec![
        Value::Integer(2),
        Value::Text("Bob".into()),
        Value::Null,
    ]))?;

    // Look up a row by primary key
    if let Some(row) = db.get("users", &Value::Integer(1))? {
        println!("Found: {:?}", row.values());
    }

    Ok(())
}
```

### Reopening an existing database

`create` fails if the directory already exists, so use `open` on later runs:

```rust
use oxidedb::Database;
use std::path::Path;

let path = Path::new("./my_db");
let mut db = if path.exists() {
    Database::open(path)?
} else {
    Database::create(path)?
};
```

### Reading all rows

```rust
for row in db.scan("users")? {
    let id = row.get(0).unwrap();
    let name = row.get(1).unwrap();
    println!("{id}: {name}");
}
```

### Updating a row

`replace` swaps in a whole new row. The primary key must stay the same.

```rust
let key = Value::Integer(1);

if let Some(mut row) = db.get("users", &key)? {
    row.set(2, Value::Integer(31))?;      // change the "age" column
    db.replace("users", &key, row)?;
}
```

### Deleting a row

```rust
let deleted = db.delete("users", &Value::Integer(2))?;
println!("deleted: {deleted}"); // false if the key did not exist
```

### Text primary keys

```rust
let schema = Schema::new(
    "settings".into(),
    vec![
        Column::new("key".into(), DataType::Text, false, true),
        Column::new("value".into(), DataType::Text, false, false),
    ],
)?;
db.create_table(schema)?;

db.insert("settings", Row::new(vec![
    Value::Text("theme".into()),
    Value::Text("dark".into()),
]))?;

let row = db.get("settings", &Value::Text("theme".into()))?;
```

### Handling errors

```rust
use oxidedb::DbError;

match db.insert("users", row) {
    Ok(()) => println!("inserted"),
    Err(DbError::DuplicateKey(key)) => println!("id {key} already exists"),
    Err(DbError::TypeMismatch { expected, actual }) => {
        println!("expected {expected}, got {actual}")
    }
    Err(e) => return Err(e.into()),
}
```

### Managing tables

```rust
println!("{:?}", db.table_names()?);   // ["settings", "users"]
db.drop_table("settings")?;
```

### Crash safety, in short

Each `insert`, `replace`, and `delete` is its own durable transaction:

1. The original versions of the pages about to change are saved to a checksummed `.journal` file, written atomically.
2. The changes are applied and synced to disk.
3. The journal is retired.

If the process dies between steps 1 and 3, the next time that table is opened the journal is replayed and the table is restored to its last committed state. A journal that fails its checksum is never applied. The table is reported as `CorruptData` instead.

### On-disk layout

```
my_db/
├── .oxidedb.lock            # exclusive lock for the whole database
├── table-7573657273.db      # one file per table (name is hex-encoded: "users")
└── table-7573657273.db.*    # transient lock/journal files, cleaned up automatically
```

## 6. Limitations

OxideDB is young (`0.x`) and deliberately small. Please read this section before using it for anything important.

- **No SQL.** There is no parser or query language. You work through the Rust API only.
- **Primary-key access only.** You can `get` by key or `scan` an entire table. There are no secondary indexes, `WHERE` filters, range queries, joins, or sorting by other columns.
- **`scan` loads the whole table into memory** as a `Vec<Row>`.
- **One primary key per table.** It must be `Integer` or `Text`, and it is required. Composite keys are not supported.
- **Rows must fit in a single 4 KB page.** There are no overflow pages, so large text values fail with `RowTooLarge` (the limit is a little under 4 KB per row).
- **The schema is fixed.** There is no `ALTER TABLE`. A table's schema must also fit in the header page.
- **No multi-operation transactions.** Each write is atomic on its own, but you cannot group several operations into one transaction or roll them back together.
- **Single-writer, single-threaded.** Methods take `&mut self`, and the database holds an exclusive lock, so a second `open` of the same directory fails while the first is alive.
- **No space reclamation.** Deleting rows does not rebalance the tree or shrink files, and there is no vacuum or compaction.
- **Every write is synced to disk**, which keeps data safe but makes bulk inserts slower than in databases with batched transactions.
- **The buffer pool caches recently used pages in memory** to reduce repeated disk reads and stages modified pages until they are synced.
- **Fixed cache size.** The buffer pool holds 64 pages and is not configurable.
- **File format is not yet stable.** It may change between `0.x` releases.
- **Not a replacement for SQLite or a production RDBMS.** It is best suited to learning, small tools, and experiments.

## 7. License

Copyright (c) 2026 Muhammad Zain

OxideDB is dual-licensed under either of:

- **MIT license**: see [LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>
- **Apache License, Version 2.0**: see [LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>
