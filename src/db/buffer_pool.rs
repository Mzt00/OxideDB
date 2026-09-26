use std::collections::HashMap;

use super::error::{DbError, Result};
use super::pager::{PAGE_SIZE, PageId, Pager};

pub const DEFAULT_CAPACITY: usize = 64;

struct CachedPage {
    bytes: [u8; PAGE_SIZE],
    dirty: bool,
    last_used: u64,
}

// Call flush or sync before dropping the pool to save dirty pages.
pub struct BufferPool {
    pager: Pager,
    pages: HashMap<PageId, CachedPage>,
    capacity: usize,
    clock: u64,
}

impl BufferPool {
    //Capacity is the maximum number of cached data pages.
    pub fn new(pager: Pager, capacity: usize) -> Result<Self> {
        if capacity == 0 {
            return Err(DbError::PageError(
                "buffer pool capacity must be greater than zero".to_string(),
            ));
        }

        Ok(Self {
            pager,
            pages: HashMap::new(),
            capacity,
            clock: 0,
        })
    }

    pub fn read_page(&mut self, page_id: PageId) -> Result<[u8; PAGE_SIZE]> {
        self.clock = self.clock.wrapping_add(1);
        if let Some(page) = self.pages.get_mut(&page_id) {
            page.last_used = self.clock;
            return Ok(page.bytes);
        }

        let bytes = self.pager.read_page(page_id)?;
        self.make_room()?;
        self.pages.insert(
            page_id,
            CachedPage {
                bytes,
                dirty: false,
                last_used: self.clock,
            },
        );
        Ok(bytes)
    }

    // Buffers a full page the file is updated on eviction or flush.
    pub fn write_page(&mut self, page_id: PageId, bytes: &[u8; PAGE_SIZE]) -> Result<()> {
        self.pager.validate_data_page_id(page_id)?;
        if !self.pages.contains_key(&page_id) {
            self.make_room()?;
        }

        self.clock = self.clock.wrapping_add(1);
        self.pages.insert(
            page_id,
            CachedPage {
                bytes: *bytes,
                dirty: true,
                last_used: self.clock,
            },
        );
        Ok(())
    }

    pub fn allocate_page(&mut self) -> Result<PageId> {
        self.pager.allocate_page()
    }

    pub fn page_count(&self) -> PageId {
        self.pager.page_count()
    }

    pub fn root_page_id(&self) -> PageId {
        self.pager.root_page_id()
    }

    pub fn set_root_page_id(&mut self, page_id: PageId) -> Result<()> {
        self.pager.validate_data_page_id(page_id)?;
        self.flush()?;
        self.pager.set_root_page_id(page_id)
    }

    pub fn flush(&mut self) -> Result<()> {
        for (&page_id, page) in &mut self.pages {
            if page.dirty {
                self.pager.write_page(page_id, &page.bytes)?;
                page.dirty = false;
            }
        }
        self.pager.flush()
    }

    pub fn sync(&mut self) -> Result<()> {
        self.flush()?;
        self.pager.sync()
    }

    fn make_room(&mut self) -> Result<()> {
        if self.pages.len() < self.capacity {
            return Ok(());
        }

        let (&page_id, page) = self
            .pages
            .iter()
            .max_by_key(|(_, page)| self.clock.wrapping_sub(page.last_used))
            .expect("a full buffer pool cannot be empty");

        if page.dirty {
            self.pager.write_page(page_id, &page.bytes)?;
        }
        self.pages.remove(&page_id);
        Ok(())
    }
}
