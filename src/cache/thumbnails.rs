use eframe::egui::TextureHandle;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// In-memory texture cache for grid thumbnails, with in-flight and failure sets.
#[derive(Default)]
pub struct ThumbnailCache {
    textures: HashMap<PathBuf, TextureHandle>,
    loading: HashSet<PathBuf>,
    failed: HashSet<PathBuf>,
}

impl ThumbnailCache {
    pub fn texture(&self, path: &Path) -> Option<&TextureHandle> {
        self.textures.get(path)
    }

    pub fn is_loading(&self, path: &Path) -> bool {
        self.loading.contains(path)
    }

    pub fn has_failed(&self, path: &Path) -> bool {
        self.failed.contains(path)
    }

    pub fn needs_request(&self, path: &Path) -> bool {
        !self.textures.contains_key(path)
            && !self.loading.contains(path)
            && !self.failed.contains(path)
    }

    pub fn begin(&mut self, path: &Path) {
        self.loading.insert(path.to_path_buf());
    }

    pub fn insert(&mut self, path: PathBuf, texture: TextureHandle) {
        self.loading.remove(&path);
        self.failed.remove(&path);
        self.textures.insert(path, texture);
    }

    pub fn fail(&mut self, path: &Path) {
        self.loading.remove(path);
        self.failed.insert(path.to_path_buf());
    }

    #[allow(dead_code)]
    pub fn remove(&mut self, path: &Path) {
        self.textures.remove(path);
        self.loading.remove(path);
        self.failed.remove(path);
    }

    pub fn retain(&mut self, paths: &HashSet<PathBuf>) {
        self.textures.retain(|path, _| paths.contains(path));
        self.loading.retain(|path| paths.contains(path));
        self.failed.retain(|path| paths.contains(path));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_lifecycle_blocks_duplicate_work() {
        let path = Path::new("a.jpg");
        let mut cache = ThumbnailCache::default();
        assert!(cache.needs_request(path));
        cache.begin(path);
        assert!(!cache.needs_request(path));
        assert!(cache.is_loading(path));
        cache.fail(path);
        assert!(!cache.needs_request(path));
        assert!(cache.has_failed(path));
        cache.remove(path);
        assert!(cache.needs_request(path));
    }
}
