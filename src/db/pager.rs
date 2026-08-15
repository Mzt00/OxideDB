use std::fs::{File, OpenOptions};

use std::io::{Read, Seek, SeekFrom, Write};

use std::path::Path;
use super::error::{DbError, Result};

pub const PAGE_SIZE: usize = 4096;
pub type PageId = u32;

pub const INVALID_PAGE_ID: PageId = u32::MAX;
const FILE_EXT: &[u8;8] = b"OXIDEDB\0";
const FORMAT_VERSION: u32 = 1;//for future version

//formatting of page 0/header in bytes:
//0-8: file type
//8-12: formt version
//12-16: page size
//16-20: root page id
//20-24:page count
//remainig will be filled as 0
//page 1 will be root
#[derive(Debug,Clone,Copy,PartialEq,Eq)]

struct FileHeader{
    root_page_id: PageId,
    page_count: PageId,
}

impl FileHeader{
    fn new()-> Self {
        Self {
            root_page_id: 1,
            page_count: 2,
        }
    }
    fn encode(&self) -> [u8; PAGE_SIZE] {
        let mut page = [0u8; PAGE_SIZE];
        page[0..8].copy_from_slice(FILE_EXT);
        page[8..12].copy_from_slice(&FORMAT_VERSION.to_le_bytes(),);
        page[12..16].copy_from_slice((PAGE_SIZE as u32).to_le_bytes(),);
        page[16..20].copy_from_slice(&self.root_page_id.to_le_bytes(),);
        page[20..24].copy_from_slice(&self.page_count.to_le_bytes(),);
        return page
    }
    fn decode(page: &[u8; PAGE_SIZE]) -> Result<Self> {
        if page[0..8] != FILE_EXT[..] {
            return Err(DbError::PageError("Invalid file header".to_string(),));
        }
        let version = read_u32(page, 8);
        if version != FORMAT_VERSION {
            return Err(DbError::PageError(format!("Invalid table format version: {}",version)));
        }

        let stored_page_size = read_u32(page, 12);

        if stored_page_size != PAGE_SIZE as u32 {
            return Err(DbError::PageError(format!("Invalid page size: expected {}, got {}",PAGE_SIZE,stored_page_size)));
        }

        let root_page_id = read_u32(page, 16);
        let page_count = read_u32(page, 20);
        Ok(Self {root_page_id,page_count,})
    }
    
}




