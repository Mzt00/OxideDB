use std::path::Path;

use super::b_tree::BPlusTree;
use super::buffer_pool::{BufferPool, DEFAULT_CAPACITY};
use super::error::Result;
use super::pager::Pager;
use super::row::Row;
use super::schema::Schema;
use super::value::Value;

pub struct Table {
    schema: Schema,
    pool: BufferPool,
    tree: BPlusTree,
}

impl Table {
    pub fn create(path: &Path, schema: Schema) -> Result<Self> {
        let pager = Pager::create_unpublished(path, &schema)?;
        let mut pool = BufferPool::new(pager, DEFAULT_CAPACITY)?;
        let tree = BPlusTree::create(&mut pool, &schema)?;
        pool.publish()?;
        Ok(Self { schema, pool, tree })
    }

    pub fn open(path: &Path) -> Result<Self> {
        let pager = Pager::open(path)?;
        let schema = pager.schema()?;
        let mut pool = BufferPool::new(pager, DEFAULT_CAPACITY)?;
        let tree = BPlusTree::open(&mut pool, &schema)?;
        Ok(Self { schema, pool, tree })
    }

    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    pub fn insert(&mut self, row: Row) -> Result<()> {
        self.pool.begin()?;
        match self.tree.insert(&mut self.pool, &self.schema, row) {
            Ok(()) => self.pool.sync(),
            Err(error) => {
                self.pool.rollback()?;
                Err(error)
            }
        }
    }

    pub fn get(&mut self, key: &Value) -> Result<Option<Row>> {
        self.tree.get(&mut self.pool, &self.schema, key)
    }

    pub fn scan(&mut self) -> Result<Vec<Row>> {
        self.tree.scan(&mut self.pool, &self.schema)
    }

    pub fn replace(&mut self, key: &Value, row: Row) -> Result<bool> {
        self.pool.begin()?;
        match self.tree.replace(&mut self.pool, &self.schema, key, row) {
            Ok(replaced) => {
                self.pool.sync()?;
                Ok(replaced)
            }
            Err(error) => {
                self.pool.rollback()?;
                Err(error)
            }
        }
    }

    pub fn delete(&mut self, key: &Value) -> Result<bool> {
        self.pool.begin()?;
        match self.tree.delete(&mut self.pool, &self.schema, key) {
            Ok(deleted) => {
                self.pool.sync()?;
                Ok(deleted)
            }
            Err(error) => {
                self.pool.rollback()?;
                Err(error)
            }
        }
    }
}
