use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub(crate) fn copy_grammar_directory(source: &Path) -> io::Result<(tempfile::TempDir, PathBuf)> {
    let temporary = tempfile::Builder::new()
        .prefix("antlr-rust-support-")
        .tempdir()?;
    let staged = temporary.path().join("grammar");
    copy_directory(source, &staged)?;
    fs::create_dir_all(staged.join("src"))?;
    Ok((temporary, fs::canonicalize(staged)?))
}

fn copy_directory(source: &Path, destination: &Path) -> io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "grammar directories with symlinks cannot be staged: {}",
                    source_path.display()
                ),
            ));
        }
        if file_type.is_dir() {
            copy_directory(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            fs::copy(source_path, destination_path)?;
        }
    }
    Ok(())
}

pub(crate) fn execute_transform(executable: &Path, staged: &Path) -> io::Result<()> {
    let output = Command::new(executable)
        .arg("--__antlr-rust-transform")
        .arg(staged)
        .current_dir(staged)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = bounded_output(&output.stderr);
    let stdout = bounded_output(&output.stdout);
    let stderr = if stderr.is_empty() {
        String::new()
    } else {
        format!("\nstderr:\n{stderr}")
    };
    let stdout = if stdout.is_empty() {
        String::new()
    } else {
        format!("\nstdout:\n{stdout}")
    };
    Err(io::Error::other(format!(
        "Rust target transform failed with {}{stderr}{stdout}",
        output.status
    )))
}

fn bounded_output(bytes: &[u8]) -> String {
    const LIMIT: usize = 64 * 1024;
    let bytes = bytes.get(..bytes.len().min(LIMIT)).unwrap_or(bytes);
    String::from_utf8_lossy(bytes).trim().to_owned()
}

pub(crate) fn top_level_files_with_extension(
    directory: &Path,
    extension: &str,
) -> io::Result<Vec<PathBuf>> {
    let mut files = fs::read_dir(directory)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension() == Some(OsStr::new(extension)))
        .collect::<Vec<_>>();
    files.sort();
    Ok(files)
}
