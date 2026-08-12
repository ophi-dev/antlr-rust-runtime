// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::error::Diagnostic;

/// Inventory returned by a completed generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Generation {
    inputs: Vec<PathBuf>,
    outputs: Vec<PathBuf>,
    warnings: Vec<String>,
    diagnostics: Vec<Diagnostic>,
}

impl Generation {
    pub(crate) const fn new(
        inputs: Vec<PathBuf>,
        outputs: Vec<PathBuf>,
        warnings: Vec<String>,
        diagnostics: Vec<Diagnostic>,
    ) -> Self {
        Self {
            inputs,
            outputs,
            warnings,
            diagnostics,
        }
    }

    /// Canonical paths of every grammar, token vocabulary, and semantic-pattern file read.
    pub fn inputs(&self) -> &[PathBuf] {
        &self.inputs
    }

    /// Paths written or confirmed current by this generation.
    pub fn outputs(&self) -> &[PathBuf] {
        &self.outputs
    }

    /// Non-fatal compiler and generator messages in CLI display form.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Structured non-fatal grammar compiler diagnostics.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Emits Cargo rebuild directives for the complete resolved input graph.
    #[allow(clippy::print_stdout)]
    pub fn emit_rerun_if_changed(&self) {
        for input in &self.inputs {
            println!("cargo::rerun-if-changed={}", input.display());
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct GeneratedArtifacts {
    files: BTreeMap<PathBuf, Vec<u8>>,
    remove_if_present: BTreeSet<PathBuf>,
}

impl GeneratedArtifacts {
    pub(crate) fn insert(
        &mut self,
        path: impl Into<PathBuf>,
        contents: impl Into<Vec<u8>>,
    ) -> io::Result<()> {
        let path = path.into();
        validate_relative_path(&path)?;
        match self.files.entry(path.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(contents.into());
                self.remove_if_present.remove(&path);
                Ok(())
            }
            Entry::Occupied(_) => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("generated artifact collision: {}", path.display()),
            )),
        }
    }

    pub(crate) fn remove_if_present(&mut self, path: impl Into<PathBuf>) -> io::Result<()> {
        let path = path.into();
        validate_relative_path(&path)?;
        if !self.files.contains_key(&path) {
            self.remove_if_present.insert(path);
        }
        Ok(())
    }

    pub(crate) fn write_to(self, output_directory: &Path) -> io::Result<Vec<PathBuf>> {
        fs::create_dir_all(output_directory)?;
        let mut outputs = Vec::with_capacity(self.files.len());
        for (relative, contents) in self.files {
            let path = output_directory.join(&relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let unchanged = fs::read(&path).is_ok_and(|existing| existing == contents);
            if !unchanged {
                fs::write(&path, contents)?;
            }
            outputs.push(path);
        }
        for relative in self.remove_if_present {
            let path = output_directory.join(relative);
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(outputs)
    }
}

fn validate_relative_path(path: &Path) -> io::Result<()> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "generated artifact path must stay below the output directory: {}",
                path.display()
            ),
        ));
    }
    Ok(())
}
