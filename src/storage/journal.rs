use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use super::files;
use crate::db::error::{DbError, Result};
use crate::db::pager::{PAGE_SIZE, PageId};

const MAGIC: &[u8; 8] = b"OXUNDO01";
const HEADER_SIZE: usize = 20;

pub(crate) fn prepare(
    path: &Path,
    file: &mut File,
    pages: &BTreeMap<PageId, [u8; PAGE_SIZE]>,
    original_len: u64,
) -> Result<()> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&original_len.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    let mut count = 0u32;
    for &id in pages.keys() {
        let offset = u64::from(id) * PAGE_SIZE as u64;
        if offset < original_len {
            let mut page = [0; PAGE_SIZE];
            file.seek(SeekFrom::Start(offset))?;
            file.read_exact(&mut page)?;
            bytes.extend_from_slice(&id.to_le_bytes());
            bytes.extend_from_slice(&page);
            count += 1;
        }
    }
    bytes[16..20].copy_from_slice(&count.to_le_bytes());
    bytes.extend_from_slice(&checksum(&bytes).to_le_bytes());
    let pending = files::sidecar(path, ".journal.pending");
    let mut journal = File::create(&pending)?;
    journal.write_all(&bytes)?;
    journal.sync_all()?;
    drop(journal);
    atomicwrites::move_atomic(&pending, &files::sidecar(path, ".journal"))?;
    Ok(())
}

pub(crate) fn finish(path: &Path) -> Result<()> {
    atomicwrites::replace_atomic(
        &files::sidecar(path, ".journal"),
        &files::sidecar(path, ".journal.done"),
    )?;
    Ok(())
}

pub(crate) fn recover(path: &Path, file: &mut File) -> Result<()> {
    let journal_path = files::sidecar(path, ".journal");
    if !journal_path.try_exists()? {
        return Ok(());
    }
    let bytes = read(&journal_path)?;
    let original_len = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
    for record in bytes[HEADER_SIZE..bytes.len() - 8].chunks_exact(PAGE_SIZE + 4) {
        let id = u32::from_le_bytes(record[..4].try_into().unwrap());
        file.seek(SeekFrom::Start(u64::from(id) * PAGE_SIZE as u64))?;
        file.write_all(&record[4..])?;
    }
    file.set_len(original_len)?;
    file.sync_all()?;
    finish(path)
}

fn read(path: &Path) -> Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    let mut header = [0; HEADER_SIZE];
    if length < (HEADER_SIZE + 8) as u64 {
        return Err(invalid());
    }
    file.read_exact(&mut header)?;
    let original_len = u64::from_le_bytes(header[8..16].try_into().unwrap());
    let count = u32::from_le_bytes(header[16..20].try_into().unwrap());
    if &header[..8] != MAGIC
        || original_len < (2 * PAGE_SIZE) as u64
        || original_len % PAGE_SIZE as u64 != 0
        || original_len / PAGE_SIZE as u64 > u64::from(u32::MAX)
        || count == 0
        || u64::from(count) > original_len / PAGE_SIZE as u64
        || length != HEADER_SIZE as u64 + u64::from(count) * (PAGE_SIZE + 4) as u64 + 8
    {
        return Err(invalid());
    }
    let mut bytes = header.to_vec();
    file.read_to_end(&mut bytes)?;
    if bytes.len() as u64 != length {
        return Err(invalid());
    }
    let end = bytes.len() - 8;
    let stored = u64::from_le_bytes(bytes[end..].try_into().unwrap());
    if stored != checksum(&bytes[..end]) {
        return Err(invalid());
    }
    let mut previous = None;
    for record in bytes[HEADER_SIZE..end].chunks_exact(PAGE_SIZE + 4) {
        let id = u32::from_le_bytes(record[..4].try_into().unwrap());
        if u64::from(id) >= original_len / PAGE_SIZE as u64
            || previous.is_some_and(|previous| previous >= id)
            || (previous.is_none() && id != 0)
        {
            return Err(invalid());
        }
        previous = Some(id);
    }
    Ok(bytes)
}

fn checksum(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
    }
    hash
}

fn invalid() -> DbError {
    DbError::CorruptData("invalid rollback journal; table was not modified".into())
}
