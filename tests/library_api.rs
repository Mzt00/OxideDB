use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use OxideDB::{Column, DataType, Database, DbError, Row, Schema, Value};

const PAGE_SIZE: usize = 4096;

fn directory(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "oxidedb-library-{label}-{}-{stamp}",
        std::process::id()
    ));
    path
}

fn schema() -> Schema {
    Schema::new(
        "items".into(),
        vec![
            Column::new("id".into(), DataType::Integer, false, true),
            Column::new("value".into(), DataType::Text, false, false),
        ],
    )
    .unwrap()
}

fn row(id: i64, value: &str) -> Row {
    Row::new(vec![Value::Integer(id), Value::Text(value.into())])
}

fn table_file(directory: &Path) -> PathBuf {
    directory.join("table-6974656d73.db")
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn install_journal(path: &Path, page_ids: &[u32]) {
    let original = fs::read(path).unwrap();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"OXUNDO01");
    bytes.extend_from_slice(&(original.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&(page_ids.len() as u32).to_le_bytes());
    for &page_id in page_ids {
        let start = page_id as usize * PAGE_SIZE;
        bytes.extend_from_slice(&page_id.to_le_bytes());
        bytes.extend_from_slice(&original[start..start + PAGE_SIZE]);
    }
    let checksum = bytes.iter().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    });
    bytes.extend_from_slice(&checksum.to_le_bytes());
    fs::write(sidecar(path, ".journal"), bytes).unwrap();
}

fn corrupt_pages(path: &Path) {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(&[0xff; PAGE_SIZE]).unwrap();
    file.seek(SeekFrom::Start(PAGE_SIZE as u64)).unwrap();
    file.write_all(&[0xaa; PAGE_SIZE]).unwrap();
    file.seek(SeekFrom::Start((2 * PAGE_SIZE) as u64)).unwrap();
    file.write_all(&[0xbb; PAGE_SIZE]).unwrap();
    file.sync_all().unwrap();
}

#[test]
fn crate_root_api_supports_crud_and_reopening() {
    let path = directory("crud");
    let mut database = Database::create(&path).unwrap();
    database.create_table(schema()).unwrap();

    database.insert("items", row(1, "first")).unwrap();
    assert_eq!(
        database.get("items", &Value::Integer(1)).unwrap(),
        Some(row(1, "first"))
    );
    assert!(matches!(
        database.insert("items", row(1, "duplicate")),
        Err(DbError::DuplicateKey(_))
    ));
    assert!(
        database
            .replace("items", &Value::Integer(1), row(1, "updated"))
            .unwrap()
    );
    assert_eq!(database.scan("items").unwrap(), vec![row(1, "updated")]);
    assert!(database.delete("items", &Value::Integer(1)).unwrap());
    assert!(database.scan("items").unwrap().is_empty());
    assert_eq!(database.table_names().unwrap(), vec!["items"]);
    drop(database);

    let mut database = Database::open(&path).unwrap();
    assert!(database.scan("items").unwrap().is_empty());
    database.drop_table("items").unwrap();
    assert!(database.table_names().unwrap().is_empty());
    drop(database);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn multiple_page_commits_survive_eviction_and_reopening() {
    let path = directory("eviction");
    let mut database = Database::create(&path).unwrap();
    database.create_table(schema()).unwrap();
    for id in 0..70 {
        database
            .insert(
                "items",
                Row::new(vec![Value::Integer(id), Value::Text("x".repeat(3000))]),
            )
            .unwrap();
    }
    drop(database);

    let mut database = Database::open(&path).unwrap();
    let rows = database.scan("items").unwrap();
    assert_eq!(rows.len(), 70);
    for (id, row) in rows.iter().enumerate() {
        assert_eq!(row.get(0), Some(&Value::Integer(id as i64)));
        assert_eq!(row.get(1), Some(&Value::Text("x".repeat(3000))));
    }
    drop(database);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn database_lock_rejects_a_second_open_and_releases_after_drop() {
    let path = directory("lock");
    let database = Database::create(&path).unwrap();
    assert!(Database::open(&path).is_err());
    drop(database);

    let database = Database::open(&path).unwrap();
    drop(database);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn recovery_restores_a_consistent_table_after_a_partial_commit() {
    let directory = directory("recovery");
    let table = table_file(&directory);
    let saved = row(7, "saved");
    let mut database = Database::create(&directory).unwrap();
    database.create_table(schema()).unwrap();
    database.insert("items", saved.clone()).unwrap();
    drop(database);

    let original = fs::read(&table).unwrap();
    install_journal(&table, &[0, 1]);
    corrupt_pages(&table);
    for suffix in [".creating", ".journal.pending", ".journal.done"] {
        fs::write(sidecar(&table, suffix), b"stale").unwrap();
    }

    let mut database = Database::open(&directory).unwrap();
    assert_eq!(database.scan("items").unwrap(), vec![saved]);
    drop(database);
    assert_eq!(fs::read(&table).unwrap(), original);
    assert!(!sidecar(&table, ".journal").exists());
    for suffix in [".creating", ".journal.pending", ".journal.done"] {
        assert!(!sidecar(&table, suffix).exists());
    }
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn corrupt_journal_fails_closed_and_abandoned_creation_is_replaced() {
    let directory = directory("journal");
    let table = table_file(&directory);
    let mut database = Database::create(&directory).unwrap();
    database.create_table(schema()).unwrap();
    drop(database);

    install_journal(&table, &[0, 1]);
    let mut journal = fs::read(sidecar(&table, ".journal")).unwrap();
    *journal.last_mut().unwrap() ^= 0xff;
    fs::write(sidecar(&table, ".journal"), journal).unwrap();
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&table)
        .unwrap();
    file.seek(SeekFrom::Start(PAGE_SIZE as u64)).unwrap();
    file.write_all(&[0x99; PAGE_SIZE]).unwrap();
    file.sync_all().unwrap();
    drop(file);

    let mut database = Database::open(&directory).unwrap();
    assert!(matches!(
        database.scan("items"),
        Err(DbError::CorruptData(_))
    ));
    drop(database);
    assert_eq!(
        &fs::read(&table).unwrap()[PAGE_SIZE..2 * PAGE_SIZE],
        &[0x99; PAGE_SIZE]
    );
    fs::remove_file(&table).unwrap();
    fs::remove_file(sidecar(&table, ".journal")).unwrap();

    let abandoned = sidecar(&table, ".creating");
    fs::write(&abandoned, b"partial table").unwrap();
    let mut database = Database::open(&directory).unwrap();
    database.create_table(schema()).unwrap();
    assert!(table.exists());
    assert!(!abandoned.exists());
    drop(database);
    fs::remove_dir_all(directory).unwrap();
}
