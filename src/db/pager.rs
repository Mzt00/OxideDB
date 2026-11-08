use super::error::{DbError, Result};
use super::schema::Schema;
use crate::storage::{files, journal};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const PAGE_SIZE: usize = 4096;
pub type PageId = u32;

pub const INVALID_PAGE_ID: PageId = u32::MAX;
const FILE_EXT: &[u8; 8] = b"OXIDEDB\0";
const HEADER_SIZE: usize = 28;
const FORMAT_VERSION: u32 = 1;
#[derive(Debug, Clone, PartialEq, Eq)]

struct FileHeader {
    root_page_id: PageId,
    page_count: PageId,
    schema: Vec<u8>,
}

impl FileHeader {
    fn new(schema: &Schema) -> Result<Self> {
        let schema = schema.encode()?;
        if schema.len() > PAGE_SIZE - HEADER_SIZE {
            return Err(DbError::InvalidSchema(format!(
                "schema requires {} bytes, header holds {}",
                schema.len(),
                PAGE_SIZE - HEADER_SIZE
            )));
        }
        Ok(Self {
            root_page_id: 1,
            page_count: 2,
            schema,
        })
    }
    fn encode(&self) -> [u8; PAGE_SIZE] {
        let mut page = [0u8; PAGE_SIZE];
        page[0..8].copy_from_slice(FILE_EXT);
        page[8..12].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
        page[12..16].copy_from_slice(&(PAGE_SIZE as u32).to_le_bytes());
        page[16..20].copy_from_slice(&self.root_page_id.to_le_bytes());
        page[20..24].copy_from_slice(&self.page_count.to_le_bytes());
        page[24..28].copy_from_slice(&(self.schema.len() as u32).to_le_bytes());
        page[HEADER_SIZE..HEADER_SIZE + self.schema.len()].copy_from_slice(&self.schema);
        return page;
    }
    fn decode(page: &[u8; PAGE_SIZE]) -> Result<Self> {
        if page[0..8] != FILE_EXT[..] {
            return Err(DbError::PageError("Invalid file header".to_string()));
        }
        let version = read_u32(page, 8);
        if version != FORMAT_VERSION {
            return Err(DbError::PageError(format!(
                "Invalid table format version: {}",
                version
            )));
        }

        let stored_page_size = read_u32(page, 12);

        if stored_page_size != PAGE_SIZE as u32 {
            return Err(DbError::PageError(format!(
                "Invalid page size: expected {}, got {}",
                PAGE_SIZE, stored_page_size
            )));
        }

        let root_page_id = read_u32(page, 16);
        let page_count = read_u32(page, 20);
        let schema_length = read_u32(page, 24) as usize;
        if schema_length == 0 || schema_length > PAGE_SIZE - HEADER_SIZE {
            return Err(DbError::CorruptData("invalid schema length".into()));
        }
        let end = HEADER_SIZE + schema_length;
        if page[end..].iter().any(|&byte| byte != 0) {
            return Err(DbError::CorruptData("nonzero header padding".into()));
        }
        let schema = page[HEADER_SIZE..end].to_vec();
        Ok(Self {
            root_page_id,
            page_count,
            schema,
        })
    }
}

pub struct Pager {
    file: File,
    path: PathBuf,
    header: FileHeader,
    original: Option<FileHeader>,
    original_len: u64,
    pending: BTreeMap<PageId, [u8; PAGE_SIZE]>,
    creating: bool,
    failed: bool,
    _lock: File,
}

impl Pager {
    pub(crate) fn create_unpublished(path: &Path, schema: &Schema) -> Result<Self> {
        let header = FileHeader::new(schema)?;
        let path = files::absolute(path)?;
        let lock = files::lock(&path)?;
        if path.try_exists()? {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "table already exists",
            )
            .into());
        }
        if files::sidecar(&path, ".journal").try_exists()? {
            return Err(DbError::RecoveryRequired(
                "journal exists but its table is missing".into(),
            ));
        }
        files::remove_if_exists(&files::sidecar(&path, ".creating"))?;
        files::remove_if_exists(&files::sidecar(&path, ".journal.pending"))?;
        files::remove_if_exists(&files::sidecar(&path, ".journal.done"))?;
        let creating_path = files::sidecar(&path, ".creating");
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(creating_path)?;
        file.try_lock().map_err(std::io::Error::from)?;
        file.write_all(&header.encode())?;
        file.write_all(&[0; PAGE_SIZE])?;
        file.sync_all()?;
        Ok(Self {
            file,
            path,
            header,
            original: None,
            original_len: 0,
            pending: BTreeMap::new(),
            creating: true,
            failed: false,
            _lock: lock,
        })
    }

    pub fn open(path: &Path) -> Result<Self> {
        let path = files::absolute(path)?;
        let lock = files::lock(&path)?;
        let mut file = OpenOptions::new().read(true).write(true).open(&path)?;
        file.try_lock().map_err(std::io::Error::from)?;
        journal::recover(&path, &mut file)?;
        files::remove_if_exists(&files::sidecar(&path, ".creating"))?;
        files::remove_if_exists(&files::sidecar(&path, ".journal.pending"))?;
        files::remove_if_exists(&files::sidecar(&path, ".journal.done"))?;
        let file_length = file.metadata()?.len();
        if file_length < PAGE_SIZE as u64 {
            return Err(DbError::PageError(
                "Table file cannot contain a header".into(),
            ));
        }
        let mut header_page = [0u8; PAGE_SIZE];
        file.seek(SeekFrom::Start(0))?;
        file.read_exact(&mut header_page)?;
        let header = FileHeader::decode(&header_page)?;
        if header.root_page_id == 0 || header.root_page_id >= header.page_count {
            return Err(DbError::PageError("Invalid root page".into()));
        }
        let expected_length = u64::from(header.page_count) * PAGE_SIZE as u64;
        if file_length != expected_length {
            return Err(DbError::PageError(format!(
                "Page count mismatch: expected {expected_length} bytes, got {file_length} bytes"
            )));
        }
        Ok(Self {
            file,
            path,
            header,
            original: None,
            original_len: file_length,
            pending: BTreeMap::new(),
            creating: false,
            failed: false,
            _lock: lock,
        })
    }

    pub fn check_healthy(&self) -> Result<()> {
        if self.failed {
            return Err(DbError::RecoveryRequired(
                "the pager must be reopened before further use".into(),
            ));
        }
        Ok(())
    }

    pub fn begin(&mut self) -> Result<()> {
        self.check_healthy()?;
        if self.original.is_some() {
            return Err(DbError::PageError(
                "a pager transaction is already active".into(),
            ));
        }
        self.original_len = self.file.metadata()?.len();
        self.original = Some(self.header.clone());
        Ok(())
    }

    pub fn start_write(&mut self) -> Result<()> {
        self.check_healthy()?;
        if self.original.is_none() {
            self.begin()?;
        }
        Ok(())
    }

    pub fn rollback(&mut self) -> Result<()> {
        self.check_healthy()?;
        if let Some(header) = self.original.take() {
            self.header = header;
            self.pending.clear();
        }
        Ok(())
    }

    pub(crate) fn publish(&mut self) -> Result<()> {
        self.check_healthy()?;
        if !self.creating {
            return Err(DbError::PageError("table is already published".into()));
        }
        self.sync()?;
        if let Err(error) =
            atomicwrites::move_atomic(&files::sidecar(&self.path, ".creating"), &self.path)
        {
            self.failed = true;
            return Err(DbError::CommitUnknown(error.to_string()));
        }
        self.creating = false;
        self.original_len = u64::from(self.header.page_count) * PAGE_SIZE as u64;
        Ok(())
    }

    pub fn root_page_id(&self) -> PageId {
        self.header.root_page_id
    }

    pub(crate) fn schema(&self) -> Result<Schema> {
        Schema::decode(&self.header.schema)
    }

    pub fn page_count(&self) -> PageId {
        self.header.page_count
    }

    pub fn allocate_page(&mut self) -> Result<PageId> {
        if self.header.page_count == INVALID_PAGE_ID {
            return Err(DbError::PageError("Page limit has been reached".into()));
        }
        self.start_write()?;
        let page_id = self.header.page_count;
        self.header.page_count += 1;
        self.pending.insert(page_id, [0; PAGE_SIZE]);
        Ok(page_id)
    }

    pub fn read_page(&mut self, page_id: PageId) -> Result<[u8; PAGE_SIZE]> {
        self.check_healthy()?;
        self.validate_data_page_id(page_id)?;
        if let Some(page) = self.pending.get(&page_id) {
            return Ok(*page);
        }
        let mut page = [0u8; PAGE_SIZE];
        self.file
            .seek(SeekFrom::Start(Self::page_offset(page_id)))?;
        self.file.read_exact(&mut page)?;
        Ok(page)
    }

    pub fn write_page(&mut self, page_id: PageId, page: &[u8; PAGE_SIZE]) -> Result<()> {
        self.validate_data_page_id(page_id)?;
        self.start_write()?;
        self.pending.insert(page_id, *page);
        Ok(())
    }

    pub fn flush(&mut self) -> Result<()> {
        self.check_healthy()?;
        self.file.flush()?;
        Ok(())
    }

    pub fn sync(&mut self) -> Result<()> {
        self.check_healthy()?;
        if self.original.is_none() {
            return self.file.sync_all().map_err(Into::into);
        }
        if self.pending.is_empty() && self.original.as_ref() == Some(&self.header) {
            self.original = None;
            return Ok(());
        }
        self.pending.insert(0, self.header.encode());
        if self.creating {
            if let Err(error) = self.apply_pending() {
                self.failed = true;
                return Err(error.into());
            }
        } else {
            if let Err(error) =
                journal::prepare(&self.path, &mut self.file, &self.pending, self.original_len)
            {
                self.failed = true;
                return Err(DbError::RecoveryRequired(error.to_string()));
            }
            if let Err(error) = self.apply_pending() {
                self.failed = true;
                return Err(DbError::CommitUnknown(error.to_string()));
            }
            if let Err(error) = journal::finish(&self.path) {
                self.failed = true;
                return Err(DbError::CommitUnknown(error.to_string()));
            }
            let _ = files::remove_if_exists(&files::sidecar(&self.path, ".journal.done"));
        }
        self.pending.clear();
        self.original = None;
        self.original_len = u64::from(self.header.page_count) * PAGE_SIZE as u64;
        Ok(())
    }

    fn apply_pending(&mut self) -> std::io::Result<()> {
        for (&page_id, page) in &self.pending {
            self.file
                .seek(SeekFrom::Start(Self::page_offset(page_id)))?;
            self.file.write_all(page)?;
        }
        self.file
            .set_len(u64::from(self.header.page_count) * PAGE_SIZE as u64)?;
        self.file.sync_all()
    }

    pub(super) fn validate_data_page_id(&self, page_id: PageId) -> Result<()> {
        if page_id == 0 || page_id >= self.header.page_count {
            return Err(DbError::InvalidPageId(page_id));
        }
        Ok(())
    }

    fn page_offset(page_id: PageId) -> u64 {
        u64::from(page_id) * PAGE_SIZE as u64
    }
}

fn read_u32(page: &[u8; PAGE_SIZE], offset: usize) -> u32 {
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&page[offset..offset + 4]);
    u32::from_le_bytes(bytes)
}
