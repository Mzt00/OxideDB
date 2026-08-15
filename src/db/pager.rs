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
        page[12..16].copy_from_slice(&(PAGE_SIZE as u32).to_le_bytes(),);
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

pub struct Pager{
    file: File,
    header: FileHeader,
}

impl Pager{
    pub fn create(path: &Path)->Result<Self>{
        let file = OpenOptions::new().read(true).write(true).create_new(true).open(path)?;
        let header = FileHeader::new();
        let mut pager = Self{
            file,
            header,
        };
        pager.write_header()?;
        let empty_root = [0u8;PAGE_SIZE];
        pager.file.seek(SeekFrom::Start(Self::page_offset(1)))?;
        pager.file.write_all(&empty_root)?;
        pager.flush()?;
        Ok(pager)
    }

    pub fn open(path:&Path)-> Result<Self>{
        let mut file = OpenOptions::new().read(true).write(true).create_new(true).open(path)?;
        let file_length = file.metadata()?.len();
        if file_length < PAGE_SIZE as u64{
            return Err(DbError::PageError("Table file cannot contain a header".to_string()));
        }
        //not using slotting initial version. will be added in later versions
        //every table will have fixed sized pages in the current one
        let mut header_page= [0u8;PAGE_SIZE];
        file.seek(SeekFrom::Start(0))?;
        file.read_exact(&mut header_page)?;
        let header = FileHeader::decode(&header_page)?;
        if header.root_page_id== 0 || header.root_page_id>=header.page_count{
            return Err(DbError::PageError("Invalid root page".to_string()));
        }
        let expected_length =
            u64::from(header.page_count) * PAGE_SIZE as u64;
        if file_length != expected_length {
            return Err(DbError::PageError(format!("Page coun Mismatch: expected {} bytes got {} bytes",expected_length,file_length)));
        }
        Ok(Self {file,header,})
    }

    pub fn root_page_id(&self)->PageId{
        self.header.root_page_id
    }
    pub fn set_root_page_id(&mut self, page_id: PageId,)->Result<()>{
        self.validate_data_page_id(page_id)?;
        self.header.root_page_id = page_id;
        self.write_header()
    }
    pub fn page_count(&self)->PageId{
        self.header.page_count
    }
    pub fn allocate_page(&mut self)->Result<PageId>{
        if self.header.page_count == INVALID_PAGE_ID{
            return Err(DbError::PageError("Page limit has been reached".to_string()));
        }
        let page_id = self.header.page_count;
        let empty_page = [0u8; PAGE_SIZE];
        self.file.seek(SeekFrom::Start(Self::page_offset(page_id),))?;
        self.file.write_all(&empty_page)?;
        self.header.page_count += 1;
        self.write_header()?;
        Ok(page_id)
    }
    pub fn read_page(
        &mut self,
        page_id: PageId,
    ) -> Result<[u8; PAGE_SIZE]> {
        self.validate_data_page_id(page_id)?;

        let mut page = [0u8; PAGE_SIZE];

        self.file.seek(SeekFrom::Start(
            Self::page_offset(page_id),
        ))?;

        self.file.read_exact(&mut page)?;

        Ok(page)
    }

    pub fn write_page(&mut self,page_id: PageId,page: &[u8; PAGE_SIZE],) -> Result<()> {
        self.validate_data_page_id(page_id)?;

        self.file.seek(SeekFrom::Start(Self::page_offset(page_id),))?;
        self.file.write_all(page)?;
        Ok(())
    }
    pub fn flush(&mut self) -> Result<()> {
        self.file.flush()?;
        Ok(())
    }
    pub fn sync(&mut self) -> Result<()> {
        self.file.sync_all()?;
        Ok(())
    }

    fn write_header(&mut self) -> Result<()> {
        let page = self.header.encode();
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(&page)?;

        Ok(())
    }

    fn validate_data_page_id(&self,page_id: PageId,) -> Result<()> {
        if page_id == 0 || page_id >= self.header.page_count
        {
            return Err(DbError::InvalidPageId(page_id));
        }

        Ok(())
    }

    fn page_offset(page_id: PageId) -> u64 {
        u64::from(page_id) * PAGE_SIZE as u64
    }
}

fn read_u32(page: &[u8; PAGE_SIZE],offset: usize,) -> u32 {
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&page[offset..offset + 4],);
    u32::from_le_bytes(bytes)
}




