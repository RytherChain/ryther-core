//! Page-based storage abstraction.
//!
//! Provides a page-based interface for persistent storage,
//! designed for async I/O integration.

use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

/// Page identifier (unique within storage).
pub type PageId = u64;

/// Fixed-size page for storage.
pub const PAGE_SIZE: usize = 4096;

/// A fixed-size page of data.
#[derive(Clone)]
pub struct Page {
    /// Page identifier
    pub id: PageId,

    /// Raw page data
    pub data: [u8; PAGE_SIZE],

    /// Whether page has been modified
    pub dirty: bool,
}

impl Page {
    /// Create a new empty page.
    pub fn new(id: PageId) -> Self {
        Self {
            id,
            data: [0u8; PAGE_SIZE],
            dirty: false,
        }
    }

    /// Create page with data.
    pub fn with_data(id: PageId, data: [u8; PAGE_SIZE]) -> Self {
        Self {
            id,
            data,
            dirty: false,
        }
    }

    /// Mark page as dirty (modified).
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Clear dirty flag.
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// Read bytes from page.
    pub fn read(&self, offset: usize, len: usize) -> Option<&[u8]> {
        if offset + len <= PAGE_SIZE {
            Some(&self.data[offset..offset + len])
        } else {
            None
        }
    }

    /// Write bytes to page.
    pub fn write(&mut self, offset: usize, data: &[u8]) -> bool {
        if offset + data.len() <= PAGE_SIZE {
            self.data[offset..offset + data.len()].copy_from_slice(data);
            self.dirty = true;
            true
        } else {
            false
        }
    }
}

impl std::fmt::Debug for Page {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Page")
            .field("id", &self.id)
            .field("dirty", &self.dirty)
            .field("data_preview", &&self.data[..16])
            .finish()
    }
}

/// Thread-safe page store with optional disk persistence.
pub struct PageStore {
    /// Pages indexed by ID (cache/in-memory)
    pages: RwLock<HashMap<PageId, Page>>,

    /// Next available page ID
    next_id: RwLock<PageId>,

    /// Optional backing file
    file: RwLock<Option<std::fs::File>>,
}

impl PageStore {
    /// Create a new in-memory page store.
    pub fn new() -> Self {
        Self {
            pages: RwLock::new(HashMap::new()),
            next_id: RwLock::new(1),
            file: RwLock::new(None),
        }
    }

    /// Open a persistent page store.
    pub fn open(path: impl AsRef<std::path::Path>) -> std::io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)?;

        let len = file.metadata()?.len();
        let page_count = len / PAGE_SIZE as u64;

        Ok(Self {
            pages: RwLock::new(HashMap::new()),
            next_id: RwLock::new(page_count + 1),
            file: RwLock::new(Some(file)),
        })
    }

    /// Allocate a new page.
    pub fn allocate(&self) -> PageId {
        let mut next_id = self.next_id.write();
        let id = *next_id;
        *next_id += 1;

        let page = Page::new(id);

        // If backed by file, extend it
        if let Some(file) = self.file.write().as_mut() {
            use std::io::{Seek, Write};
            let offset = (id - 1) * PAGE_SIZE as u64;
            if let Ok(_) = file.seek(std::io::SeekFrom::Start(offset)) {
                let _ = file.write_all(&page.data);
            }
        }

        self.pages.write().insert(id, page);

        id
    }

    /// Read a page by ID.
    pub fn read_page(&self, id: PageId) -> Option<Page> {
        // Check cache first
        if let Some(page) = self.pages.read().get(&id) {
            return Some(page.clone());
        }

        // Read from disk
        let mut file_guard = self.file.write();
        if let Some(file) = file_guard.as_mut() {
            use std::io::{Read, Seek};
            let offset = (id - 1) * PAGE_SIZE as u64;

            if file.seek(std::io::SeekFrom::Start(offset)).is_ok() {
                let mut data = [0u8; PAGE_SIZE];
                if file.read_exact(&mut data).is_ok() {
                    let page = Page::with_data(id, data);
                    // Add to cache
                    // Note: This requires dropping file_guard or avoiding deadlock if we want to write to cache.
                    // But we hold write lock on file, cache is separate RwLock.
                    // To avoid holding file lock while waiting for cache lock:
                    // Drop file lock? No, we need data.
                    // Just return here and let caller handle cache, or insert now.
                    // We can try_write to cache or just return.
                    // For simplicity, we just return the page.
                    return Some(page);
                }
            }
        }

        None
    }

    /// Write a page.
    pub fn write_page(&self, page: Page) {
        // Update cache
        self.pages.write().insert(page.id, page.clone());

        // Write to disk
        if let Some(file) = self.file.write().as_mut() {
            use std::io::{Seek, Write};
            let offset = (page.id - 1) * PAGE_SIZE as u64;
            if file.seek(std::io::SeekFrom::Start(offset)).is_ok() {
                let _ = file.write_all(&page.data);
            }
        }
    }

    /// Check if page exists.
    pub fn exists(&self, id: PageId) -> bool {
        self.pages.read().contains_key(&id) || {
            // Check if ID is within file bounds
            if let Some(file) = self.file.read().as_ref() {
                let len = file.metadata().map(|m| m.len()).unwrap_or(0);
                (id * PAGE_SIZE as u64) <= len
            } else {
                false
            }
        }
    }

    /// Free a page.
    pub fn free(&self, id: PageId) -> bool {
        // Just remove from cache
        self.pages.write().remove(&id).is_some()
        // In simple linear storage, we don't reclaim space easily without a freelist.
    }

    /// Get all dirty pages.
    pub fn dirty_pages(&self) -> Vec<PageId> {
        self.pages
            .read()
            .iter()
            .filter(|(_, p)| p.dirty)
            .map(|(id, _)| *id)
            .collect()
    }

    /// Flush all dirty pages (mark clean).
    pub fn flush(&self) {
        let mut pages = self.pages.write();

        // If persistent, ensure fsync
        if let Some(file) = self.file.read().as_ref() {
            let _ = file.sync_all();
        }

        for page in pages.values_mut() {
            page.mark_clean();
        }
    }

    /// Total pages allocated.
    pub fn page_count(&self) -> usize {
        // Approximation
        *self.next_id.read() as usize - 1
    }
}

impl Default for PageStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Async page store wrapper for non-blocking I/O.
///
/// Wraps PageStore with async interface for use with tokio.
pub struct AsyncPageStore {
    inner: Arc<PageStore>,
}

impl AsyncPageStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(PageStore::new()),
        }
    }

    /// Allocate a new page asynchronously.
    pub async fn allocate(&self) -> PageId {
        let store = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || store.allocate())
            .await
            .unwrap()
    }

    /// Read a page asynchronously.
    pub async fn read_page(&self, id: PageId) -> Option<Page> {
        let store = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || store.read_page(id))
            .await
            .unwrap()
    }

    /// Write a page asynchronously.
    pub async fn write_page(&self, page: Page) {
        let store = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || store.write_page(page))
            .await
            .unwrap()
    }

    /// Flush pages asynchronously.
    pub async fn flush(&self) {
        let store = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || store.flush())
            .await
            .unwrap()
    }
}

impl Default for AsyncPageStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_page_allocation() {
        let store = PageStore::new();

        let id1 = store.allocate();
        let id2 = store.allocate();
        let id3 = store.allocate();

        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
        assert_eq!(id3, 3);
        assert_eq!(store.page_count(), 3);
    }

    #[test]
    fn test_page_read_write() {
        let store = PageStore::new();
        let id = store.allocate();

        let mut page = store.read_page(id).unwrap();
        page.write(0, b"Hello, Ryther!");
        store.write_page(page);

        let page = store.read_page(id).unwrap();
        assert_eq!(page.read(0, 14).unwrap(), b"Hello, Ryther!");
    }

    #[test]
    fn test_dirty_tracking() {
        let store = PageStore::new();
        let id = store.allocate();

        // New page isn't dirty
        assert!(store.dirty_pages().is_empty());

        // Modify page
        let mut page = store.read_page(id).unwrap();
        page.write(0, b"test");
        store.write_page(page);

        // Now it's dirty
        assert_eq!(store.dirty_pages(), vec![id]);

        // Flush clears dirty
        store.flush();
        assert!(store.dirty_pages().is_empty());
    }

    #[test]
    fn test_page_free() {
        let store = PageStore::new();
        let id = store.allocate();

        assert!(store.exists(id));
        assert!(store.free(id));
        assert!(!store.exists(id));
    }

    #[tokio::test]
    async fn test_async_page_store() {
        let store = AsyncPageStore::new();

        let id = store.allocate().await;
        let page = store.read_page(id).await;

        assert!(page.is_some());
    }
}
