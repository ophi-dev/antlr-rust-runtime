use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::frontend::{SourceFile, SourceId};

#[derive(Debug, Default)]
pub(crate) struct SourceSet {
    files: Vec<SourceFile>,
    canonical_paths: Vec<PathBuf>,
    by_canonical_path: BTreeMap<PathBuf, SourceId>,
}

impl SourceSet {
    pub(crate) fn insert(
        &mut self,
        canonical_path: PathBuf,
        file: SourceFile,
    ) -> Result<SourceId, SourceId> {
        if let Some(id) = self.by_canonical_path.get(&canonical_path) {
            return Err(*id);
        }
        let id = file.id();
        debug_assert_eq!(id.index(), self.files.len());
        self.by_canonical_path.insert(canonical_path.clone(), id);
        self.canonical_paths.push(canonical_path);
        self.files.push(file);
        Ok(id)
    }

    pub(crate) fn next_id(&self) -> SourceId {
        SourceId::new(u32::try_from(self.files.len()).expect("source count exceeds u32"))
    }

    pub(crate) fn get(&self, id: SourceId) -> Option<&SourceFile> {
        self.files.get(id.index())
    }

    pub(crate) fn canonical_path(&self, id: SourceId) -> Option<&Path> {
        self.canonical_paths.get(id.index()).map(PathBuf::as_path)
    }

    pub(crate) fn id_for_canonical_path(&self, path: &Path) -> Option<SourceId> {
        self.by_canonical_path.get(path).copied()
    }

    pub(crate) fn iter(&self) -> impl ExactSizeIterator<Item = &SourceFile> {
        self.files.iter()
    }

    pub(crate) const fn len(&self) -> usize {
        self.files.len()
    }

    pub(crate) const fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}
