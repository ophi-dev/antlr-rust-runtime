use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::frontend::{SourceFile, SourceId};

#[derive(Debug, Default)]
pub(crate) struct SourceSet {
    files: Vec<Option<SourceFile>>,
    failed_sources: BTreeMap<SourceId, FailedSource>,
    canonical_paths: Vec<PathBuf>,
    by_canonical_path: BTreeMap<PathBuf, SourceId>,
}

#[derive(Debug)]
struct FailedSource {
    logical_path: PathBuf,
    text: Box<str>,
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
        self.files.push(Some(file));
        Ok(id)
    }

    pub(crate) fn insert_failed(
        &mut self,
        canonical_path: PathBuf,
        source: SourceId,
        logical_path: PathBuf,
        text: String,
    ) -> Result<(), SourceId> {
        if let Some(id) = self.by_canonical_path.get(&canonical_path) {
            return Err(*id);
        }
        debug_assert_eq!(source.index(), self.files.len());
        self.by_canonical_path
            .insert(canonical_path.clone(), source);
        self.canonical_paths.push(canonical_path);
        self.files.push(None);
        self.failed_sources.insert(
            source,
            FailedSource {
                logical_path,
                text: text.into_boxed_str(),
            },
        );
        Ok(())
    }

    pub(crate) fn next_id(&self) -> SourceId {
        SourceId::new(u32::try_from(self.files.len()).expect("source count exceeds u32"))
    }

    pub(crate) fn get(&self, id: SourceId) -> Option<&SourceFile> {
        self.files.get(id.index()).and_then(Option::as_ref)
    }

    pub(crate) fn logical_path(&self, id: SourceId) -> Option<&Path> {
        self.get(id).map(SourceFile::logical_path).or_else(|| {
            self.failed_sources
                .get(&id)
                .map(|source| source.logical_path.as_path())
        })
    }

    pub(crate) fn line_column(&self, id: SourceId, byte: u32) -> Option<(usize, usize)> {
        if let Some(file) = self.get(id) {
            return file.line_column(byte);
        }
        self.failed_sources
            .get(&id)
            .and_then(|source| line_column(&source.text, byte))
    }

    pub(crate) fn canonical_path(&self, id: SourceId) -> Option<&Path> {
        self.canonical_paths.get(id.index()).map(PathBuf::as_path)
    }

    pub(crate) fn id_for_canonical_path(&self, path: &Path) -> Option<SourceId> {
        self.by_canonical_path.get(path).copied()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &SourceFile> {
        self.files.iter().filter_map(Option::as_ref)
    }

    pub(crate) fn len(&self) -> usize {
        self.files.len() - self.failed_sources.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn line_column(text: &str, byte: u32) -> Option<(usize, usize)> {
    let byte = byte as usize;
    if byte > text.len() || !text.is_char_boundary(byte) {
        return None;
    }
    let prefix = &text[..byte];
    let line = prefix
        .bytes()
        .filter(|character| *character == b'\n')
        .count()
        + 1;
    let line_start = prefix.rfind('\n').map_or(0, |index| index + 1);
    let column = text[line_start..byte].chars().count();
    Some((line, column))
}
