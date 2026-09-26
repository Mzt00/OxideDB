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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, OpenOptions};
    use std::io::{Seek, SeekFrom, Write};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_FILE: AtomicUsize = AtomicUsize::new(0);

    struct TestFile(PathBuf);

    impl TestFile {
        fn new() -> Self {
            let id = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
            Self(std::env::temp_dir().join(format!(
                "oxidedb-buffer-pool-{}-{id}.db",
                std::process::id()
            )))
        }

        fn pool(&self, capacity: usize) -> BufferPool {
            BufferPool::new(Pager::create(&self.0).unwrap(), capacity).unwrap()
        }
    }

    impl Drop for TestFile {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    #[test]
    fn writes_stay_cached_until_flush() {
        let file = TestFile::new();
        let mut pool = file.pool(DEFAULT_CAPACITY);
        let page = [7; PAGE_SIZE];

        pool.write_page(1, &page).unwrap();
        assert_eq!(pool.read_page(1).unwrap(), page);
        let mut disk = Pager::open(&file.0).unwrap();
        assert_eq!(disk.read_page(1).unwrap(), [0; PAGE_SIZE]);

        pool.flush().unwrap();
        assert_eq!(disk.read_page(1).unwrap(), page);
        assert!(!pool.pages[&1].dirty);
        assert_eq!(pool.pages.len(), 1);
    }

    #[test]
    fn reads_and_writes_refresh_lru_and_dirty_eviction_persists() {
        let file = TestFile::new();
        let mut pool = file.pool(2);
        let second = pool.allocate_page().unwrap();
        let third = pool.allocate_page().unwrap();

        pool.write_page(1, &[1; PAGE_SIZE]).unwrap();
        pool.write_page(second, &[2; PAGE_SIZE]).unwrap();
        pool.read_page(1).unwrap();
        pool.read_page(third).unwrap();
        assert!(pool.pages.contains_key(&1));
        assert!(!pool.pages.contains_key(&second));
        assert_eq!(pool.pages.len(), 2);
        assert_eq!(
            Pager::open(&file.0).unwrap().read_page(second).unwrap(),
            [2; PAGE_SIZE]
        );

        pool.write_page(1, &[9; PAGE_SIZE]).unwrap();
        pool.read_page(second).unwrap();
        assert!(!pool.pages.contains_key(&third));
        assert_eq!(pool.read_page(1).unwrap(), [9; PAGE_SIZE]);
        pool.sync().unwrap();
    }

    #[test]
    fn clean_eviction_does_not_write_and_root_is_not_pinned() {
        let file = TestFile::new();
        let mut pool = file.pool(1);
        let second = pool.allocate_page().unwrap();
        pool.read_page(1).unwrap();

        let mut disk = Pager::open(&file.0).unwrap();
        disk.write_page(1, &[5; PAGE_SIZE]).unwrap();
        assert_eq!(pool.read_page(1).unwrap(), [0; PAGE_SIZE]);
        pool.read_page(second).unwrap();
        assert!(!pool.pages.contains_key(&1));
        assert_eq!(pool.read_page(1).unwrap(), [5; PAGE_SIZE]);
    }

    #[test]
    fn invalid_requests_leave_dirty_pages_cached() {
        let file = TestFile::new();
        let mut pool = file.pool(1);
        pool.write_page(1, &[3; PAGE_SIZE]).unwrap();

        for page_id in [0, pool.page_count(), u32::MAX] {
            assert!(matches!(
                pool.read_page(page_id),
                Err(DbError::InvalidPageId(_))
            ));
            assert!(matches!(
                pool.write_page(page_id, &[0; PAGE_SIZE]),
                Err(DbError::InvalidPageId(_))
            ));
            assert!(matches!(
                pool.set_root_page_id(page_id),
                Err(DbError::InvalidPageId(_))
            ));
        }
        assert!(pool.pages[&1].dirty);
        assert_eq!(pool.read_page(1).unwrap(), [3; PAGE_SIZE]);
        assert_eq!(pool.root_page_id(), 1);
        assert_eq!(
            Pager::open(&file.0).unwrap().read_page(1).unwrap(),
            [0; PAGE_SIZE]
        );
        pool.flush().unwrap();
    }

    #[test]
    fn failed_read_keeps_dirty_victim() {
        let file = TestFile::new();
        let mut pool = file.pool(1);
        let second = pool.allocate_page().unwrap();
        pool.write_page(1, &[6; PAGE_SIZE]).unwrap();

        let mut disk = OpenOptions::new().write(true).open(&file.0).unwrap();
        disk.set_len(u64::from(second) * PAGE_SIZE as u64).unwrap();
        assert!(matches!(pool.read_page(second), Err(DbError::Io(_))));
        assert_eq!(pool.pages.len(), 1);
        assert!(pool.pages[&1].dirty);
        assert_eq!(pool.read_page(1).unwrap(), [6; PAGE_SIZE]);

        disk.seek(SeekFrom::End(0)).unwrap();
        disk.write_all(&[0; PAGE_SIZE]).unwrap();
        pool.sync().unwrap();
    }

    #[test]
    fn root_change_and_sync_survive_reopening() {
        let file = TestFile::new();
        let mut pool = file.pool(1);
        let root = pool.allocate_page().unwrap();
        assert_eq!(pool.page_count(), 3);
        assert!(pool.pages.is_empty());
        assert_eq!(pool.read_page(root).unwrap(), [0; PAGE_SIZE]);

        pool.write_page(root, &[8; PAGE_SIZE]).unwrap();
        pool.set_root_page_id(root).unwrap();
        let mut disk = Pager::open(&file.0).unwrap();
        assert_eq!(disk.root_page_id(), root);
        assert_eq!(disk.read_page(root).unwrap(), [8; PAGE_SIZE]);
        drop(disk);

        pool.write_page(root, &[9; PAGE_SIZE]).unwrap();
        pool.sync().unwrap();
        drop(pool);
        let mut disk = Pager::open(&file.0).unwrap();
        assert_eq!(disk.read_page(root).unwrap(), [9; PAGE_SIZE]);
    }

    #[test]
    fn rejects_zero_capacity() {
        let file = TestFile::new();
        assert!(matches!(
            BufferPool::new(Pager::create(&file.0).unwrap(), 0),
            Err(DbError::PageError(_))
        ));
    }
}
