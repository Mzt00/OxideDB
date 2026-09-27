use std::cmp::Ordering;
use std::collections::HashSet;

use super::buffer_pool::BufferPool;
use super::error::{DbError, Result};
use super::pager::{INVALID_PAGE_ID, PAGE_SIZE, PageId};
use super::row::Row;
use super::schema::Schema;
use super::value::{DataType, Value};

const HEADER_SIZE: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Integer(i64),
    Text(String),
}

impl Key {
    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::Integer(value) => Ok(Self::Integer(*value)),
            Value::Text(value) => Ok(Self::Text(value.clone())),
            _ => Err(DbError::InvalidRow("primary key must be INTEGER or TEXT".into())),
        }
    }

    fn encoded_len(&self) -> usize {
        match self {
            Self::Integer(_) => 8,
            Self::Text(value) => value.len(),
        }
    }

    fn decode(bytes: &[u8], data_type: DataType) -> Result<Self> {
        match data_type {
            DataType::Integer => {
                let bytes = bytes.try_into().map_err(|_| {
                    DbError::CorruptData("integer key must contain eight bytes".into())
                })?;
                Ok(Self::Integer(i64::from_le_bytes(bytes)))
            }
            DataType::Text => {
                let value = std::str::from_utf8(bytes)
                    .map_err(|_| DbError::CorruptData("invalid UTF-8 key".into()))?;
                Ok(Self::Text(value.to_string()))
            }
            _ => Err(DbError::CorruptData("invalid primary key type".into())),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Node {
    keys: Vec<Key>,
    children: Vec<PageId>,
    rows: Vec<Row>,
    next_leaf: PageId,
}

impl Node {
    fn empty() -> Self {
        Self {
            keys: Vec::new(),
            children: Vec::new(),
            rows: Vec::new(),
            next_leaf: INVALID_PAGE_ID,
        }
    }

    fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }

    fn validate(&self, schema: &Schema) -> Result<()> {
        if self.is_leaf() {
            if !self.keys.is_empty() {
                return Err(DbError::PageError("leaf cannot contain separator keys".into()));
            }
            if self.next_leaf == 0 {
                return Err(DbError::InvalidPageId(0));
            }
            let mut previous = None;
            for row in &self.rows {
                row.validate(schema)?;
                let key = Key::from_value(row.primary_key(schema)?)?;
                if previous.as_ref().is_some_and(|previous| previous >= &key) {
                    return Err(DbError::PageError("leaf keys must be strictly increasing".into()));
                }
                previous = Some(key);
            }
        } else {
            if !self.rows.is_empty() || self.next_leaf != INVALID_PAGE_ID {
                return Err(DbError::PageError("internal node cannot contain rows or a leaf link".into()));
            }
            if self.children.len() != self.keys.len() + 1 {
                return Err(DbError::PageError("internal node must have one more child than keys".into()));
            }
            let mut children = HashSet::new();
            for &child in &self.children {
                if child == 0 || child == INVALID_PAGE_ID {
                    return Err(DbError::InvalidPageId(child));
                }
                if !children.insert(child) {
                    return Err(DbError::PageError("duplicate child page".into()));
                }
            }
            let key_type = schema.primary_key_column().data_type();
            for key in &self.keys {
                if !matches!((key, key_type),
                    (Key::Integer(_), DataType::Integer) | (Key::Text(_), DataType::Text)) {
                    return Err(DbError::PageError("key type does not match schema".into()));
                }
            }
            if self.keys.windows(2).any(|pair| pair[0] >= pair[1]) {
                return Err(DbError::PageError("separator keys must be strictly increasing".into()));
            }
        }
        Ok(())
    }

    fn entry_sizes(&self, schema: &Schema) -> Result<Vec<usize>> {
        if self.is_leaf() {
            self.rows.iter().map(|row| Ok(2 + row.encode(schema)?.len())).collect()
        } else {
            Ok(self.keys.iter().map(|key| 2 + key.encoded_len() + 4).collect())
        }
    }

    fn encoded_len(&self, schema: &Schema) -> Result<usize> {
        self.validate(schema)?;
        let base = HEADER_SIZE + if self.is_leaf() { 0 } else { 4 };
        self.entry_sizes(schema)?.into_iter().try_fold(base, |size, entry| {
            size.checked_add(entry).ok_or_else(|| DbError::PageError("node size overflow".into()))
        })
    }

    fn encode(&self, schema: &Schema) -> Result<[u8; PAGE_SIZE]> {
        let size = self.encoded_len(schema)?;
        if size > PAGE_SIZE {
            return Err(DbError::PageError(format!("node requires {size} bytes, page holds {PAGE_SIZE}")));
        }
        let count = if self.is_leaf() { self.rows.len() } else { self.keys.len() };
        // Header: entry count (u16), child count (u16), next leaf (u32).
        let mut bytes = Vec::with_capacity(size);
        bytes.extend_from_slice(&(count as u16).to_le_bytes());
        bytes.extend_from_slice(&(self.children.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&self.next_leaf.to_le_bytes());
        for child in &self.children {
            bytes.extend_from_slice(&child.to_le_bytes());
        }
        if self.is_leaf() {
            for row in &self.rows {
                let row = row.encode(schema)?;
                bytes.extend_from_slice(&(row.len() as u16).to_le_bytes());
                bytes.extend_from_slice(&row);
            }
        } else {
            for key in &self.keys {
                bytes.extend_from_slice(&(key.encoded_len() as u16).to_le_bytes());
                match key {
                    Key::Integer(value) => bytes.extend_from_slice(&value.to_le_bytes()),
                    Key::Text(value) => bytes.extend_from_slice(value.as_bytes()),
                }
            }
        }
        let mut page = [0; PAGE_SIZE];
        page[..bytes.len()].copy_from_slice(&bytes);
        Ok(page)
    }

    fn decode(page: &[u8; PAGE_SIZE], schema: &Schema) -> Result<Self> {
        let mut bytes = &page[..];
        let count = usize::from(read_u16(&mut bytes)?);
        let child_count = usize::from(read_u16(&mut bytes)?);
        let mut node = Self::empty();
        node.next_leaf = read_u32(&mut bytes)?;
        if child_count != 0 && child_count != count + 1 {
            return Err(DbError::CorruptData("invalid child count".into()));
        }
        for _ in 0..child_count {
            node.children.push(read_u32(&mut bytes)?);
        }
        for _ in 0..count {
            let length = usize::from(read_u16(&mut bytes)?);
            let entry = take(&mut bytes, length)?;
            if node.is_leaf() {
                node.rows.push(Row::decode(entry, schema).map_err(|error| {
                    DbError::CorruptData(format!("invalid leaf row: {error}"))
                })?);
            } else {
                node.keys.push(Key::decode(entry, schema.primary_key_column().data_type())?);
            }
        }
        if bytes.iter().any(|&byte| byte != 0) {
            return Err(DbError::CorruptData("nonzero node padding".into()));
        }
        node.validate(schema).map_err(|error| DbError::CorruptData(format!("invalid node: {error}")))?;
        Ok(node)
    }

    fn search_rows(&self, schema: &Schema, key: &Key) -> Result<std::result::Result<usize, usize>> {
        let mut low = 0;
        let mut high = self.rows.len();
        while low < high {
            let middle = low + (high - low) / 2;
            let current = Key::from_value(self.rows[middle].primary_key(schema)?)?;
            match current.cmp(key) {
                Ordering::Less => low = middle + 1,
                Ordering::Greater => high = middle,
                Ordering::Equal => return Ok(Ok(middle)),
            }
        }
        Ok(Err(low))
    }

    fn split(&mut self, schema: &Schema) -> Result<(Key, Self)> {
        let sizes = self.entry_sizes(schema)?;
        let total: usize = sizes.iter().sum();
        let mut left = 0;
        let mut best = None;
        for (index, size) in sizes.iter().copied().enumerate() {
            let (split, right) = if self.is_leaf() {
                left += size;
                if index + 1 == sizes.len() {
                    break;
                }
                (index + 1, total - left)
            } else {
                (index, total - left - size)
            };
            let difference = left.abs_diff(right);
            if best.is_none_or(|(_, previous)| difference < previous) {
                best = Some((split, difference));
            }
            if !self.is_leaf() {
                left += size;
            }
        }
        let (split, _) = best.ok_or_else(|| DbError::PageError("entry cannot fit in a page".into()))?;
        let mut right = Self::empty();
        let separator;
        if self.is_leaf() {
            right.rows = self.rows.split_off(split);
            right.next_leaf = self.next_leaf;
            separator = Key::from_value(right.rows[0].primary_key(schema)?)?;
        } else {
            right.keys = self.keys.split_off(split + 1);
            separator = self.keys.pop().ok_or_else(|| DbError::PageError("missing separator".into()))?;
            right.children = self.children.split_off(split + 1);
        }
        Ok((separator, right))
    }
}

/// The root page ID stays fixed as the tree grows. Persist changes with pool.sync().
pub(crate) struct BPlusTree {
    root_page_id: PageId,
}

impl BPlusTree {
    pub(crate) fn create(pool: &mut BufferPool, schema: &Schema) -> Result<Self> {
        let page = Node::empty().encode(schema)?;
        let root_page_id = pool.root_page_id();
        pool.write_page(root_page_id, &page)?;
        Ok(Self { root_page_id })
    }

    pub(crate) fn open(pool: &mut BufferPool, schema: &Schema) -> Result<Self> {
        let root_page_id = pool.root_page_id();
        Self::read_node(pool, schema, root_page_id)?;
        Ok(Self { root_page_id })
    }

    pub(crate) fn get(&self, pool: &mut BufferPool, schema: &Schema, value: &Value) -> Result<Option<Row>> {
        let key = Self::key(schema, value)?;
        let (_, node) = self.find_leaf(pool, schema, Some(&key), &mut Vec::new())?;
        Ok(node.search_rows(schema, &key)?.ok().map(|index| node.rows[index].clone()))
    }

    pub(crate) fn scan(&self, pool: &mut BufferPool, schema: &Schema) -> Result<Vec<Row>> {
        let (mut page_id, mut node) = self.find_leaf(pool, schema, None, &mut Vec::new())?;
        let mut visited = HashSet::new();
        let mut rows = Vec::new();
        let mut previous = None;
        loop {
            if !visited.insert(page_id) || !node.is_leaf() {
                return Err(DbError::CorruptData("invalid leaf chain".into()));
            }
            if let Some(first) = node.rows.first() {
                let first = Key::from_value(first.primary_key(schema)?)?;
                if previous.as_ref().is_some_and(|key| key >= &first) {
                    return Err(DbError::CorruptData("leaf chain is out of order".into()));
                }
            }
            if let Some(last) = node.rows.last() {
                previous = Some(Key::from_value(last.primary_key(schema)?)?);
            }
            rows.extend(node.rows);
            if node.next_leaf == INVALID_PAGE_ID {
                return Ok(rows);
            }
            page_id = node.next_leaf;
            node = Self::read_node(pool, schema, page_id)?;
        }
    }

    pub(crate) fn insert(&mut self, pool: &mut BufferPool, schema: &Schema, row: Row) -> Result<()> {
        let key = Self::check_row(schema, &row)?;
        let mut path = Vec::new();
        let (page_id, mut node) = self.find_leaf(pool, schema, Some(&key), &mut path)?;
        let index = match node.search_rows(schema, &key)? {
            Ok(_) => return Err(DbError::DuplicateKey(row.primary_key(schema)?.to_string())),
            Err(index) => index,
        };
        node.rows.insert(index, row);
        self.save_path(pool, schema, page_id, node, path)
    }

    pub(crate) fn replace(&mut self, pool: &mut BufferPool, schema: &Schema, value: &Value, row: Row) -> Result<bool> {
        let key = Self::key(schema, value)?;
        if Self::check_row(schema, &row)? != key {
            return Err(DbError::InvalidRow("replacement cannot change the primary key".into()));
        }
        let mut path = Vec::new();
        let (page_id, mut node) = self.find_leaf(pool, schema, Some(&key), &mut path)?;
        let Ok(index) = node.search_rows(schema, &key)? else { return Ok(false); };
        node.rows[index] = row;
        self.save_path(pool, schema, page_id, node, path)?;
        Ok(true)
    }

    pub(crate) fn delete(&mut self, pool: &mut BufferPool, schema: &Schema, value: &Value) -> Result<bool> {
        let key = Self::key(schema, value)?;
        let (page_id, mut node) = self.find_leaf(pool, schema, Some(&key), &mut Vec::new())?;
        let Ok(index) = node.search_rows(schema, &key)? else { return Ok(false); };
        // ponytail: retain sparse pages and separator bounds; add merging when page reuse is needed.
        node.rows.remove(index);
        pool.write_page(page_id, &node.encode(schema)?)?;
        Ok(true)
    }

    fn key(schema: &Schema, value: &Value) -> Result<Key> {
        schema.primary_key_column().validate_value(value)?;
        Key::from_value(value)
    }

    fn check_row(schema: &Schema, row: &Row) -> Result<Key> {
        let size = row.encode(schema)?.len();
        if size > PAGE_SIZE - HEADER_SIZE - 2 {
            return Err(DbError::RowTooLarge { row_size: size, page_size: PAGE_SIZE - HEADER_SIZE - 2 });
        }
        let key = Self::key(schema, row.primary_key(schema)?)?;
        if key.encoded_len() > PAGE_SIZE - HEADER_SIZE - 10 {
            return Err(DbError::InvalidRow("primary key cannot fit in an internal node".into()));
        }
        Ok(key)
    }

    fn read_node(pool: &mut BufferPool, schema: &Schema, page_id: PageId) -> Result<Node> {
        let node = Node::decode(&pool.read_page(page_id)?, schema)?;
        for &link in node.children.iter().chain(
            std::iter::once(&node.next_leaf).filter(|&&link| link != INVALID_PAGE_ID),
        ) {
            if link == page_id || link >= pool.page_count() {
                return Err(DbError::CorruptData("invalid node page link".into()));
            }
        }
        Ok(node)
    }

    fn find_leaf(&self, pool: &mut BufferPool, schema: &Schema, key: Option<&Key>, path: &mut Vec<(PageId, usize)>) -> Result<(PageId, Node)> {
        let mut page_id = self.root_page_id;
        let mut visited = HashSet::new();
        loop {
            if !visited.insert(page_id) {
                return Err(DbError::CorruptData("cycle in tree children".into()));
            }
            let node = Self::read_node(pool, schema, page_id)?;
            if node.is_leaf() {
                return Ok((page_id, node));
            }
            let index = key.map_or(0, |key| node.keys.partition_point(|separator| separator <= key));
            path.push((page_id, index));
            page_id = node.children[index];
        }
    }

    fn save_node(pool: &mut BufferPool, schema: &Schema, page_id: PageId, node: Node) -> Result<Vec<(Key, PageId)>> {
        let mut nodes = vec![node];
        let mut separators = Vec::new();
        let mut index = 0;
        while index < nodes.len() {
            if nodes[index].encoded_len(schema)? > PAGE_SIZE {
                let (key, right) = nodes[index].split(schema)?;
                nodes.insert(index + 1, right);
                separators.insert(index, key);
            } else {
                index += 1;
            }
        }
        let mut pages = vec![page_id];
        for _ in 1..nodes.len() {
            pages.push(pool.allocate_page()?);
        }
        if nodes[0].is_leaf() {
            for index in 0..nodes.len() - 1 {
                nodes[index].next_leaf = pages[index + 1];
            }
        }
        for index in (0..nodes.len()).rev() {
            pool.write_page(pages[index], &nodes[index].encode(schema)?)?;
        }
        Ok(separators.into_iter().zip(pages.into_iter().skip(1)).collect())
    }

    fn save_path(&self, pool: &mut BufferPool, schema: &Schema, page_id: PageId, node: Node, mut path: Vec<(PageId, usize)>) -> Result<()> {
        let mut separators = Self::save_node(pool, schema, page_id, node)?;
        while let Some((parent_id, index)) = path.pop() {
            if separators.is_empty() {
                return Ok(());
            }
            let mut parent = Self::read_node(pool, schema, parent_id)?;
            for (offset, (key, child)) in separators.into_iter().enumerate() {
                parent.keys.insert(index + offset, key);
                parent.children.insert(index + offset + 1, child);
            }
            separators = Self::save_node(pool, schema, parent_id, parent)?;
        }
        while !separators.is_empty() {
            let left = pool.read_page(self.root_page_id)?;
            let left_id = pool.allocate_page()?;
            pool.write_page(left_id, &left)?;
            let mut root = Node::empty();
            root.children.push(left_id);
            for (key, child) in separators {
                root.keys.push(key);
                root.children.push(child);
            }
            separators = Self::save_node(pool, schema, self.root_page_id, root)?;
        }
        Ok(())
    }
}

fn take<'a>(bytes: &mut &'a [u8], length: usize) -> Result<&'a [u8]> {
    if length > bytes.len() {
        return Err(DbError::CorruptData("node entry exceeds page boundary".into()));
    }
    let (value, remaining) = bytes.split_at(length);
    *bytes = remaining;
    Ok(value)
}

fn read_u16(bytes: &mut &[u8]) -> Result<u16> {
    let value = take(bytes, 2)?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn read_u32(bytes: &mut &[u8]) -> Result<u32> {
    let value = take(bytes, 4)?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}
