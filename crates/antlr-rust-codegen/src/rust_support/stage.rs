use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const OUTPUT_LIMIT: usize = 64 * 1024;

pub(crate) fn copy_grammar_directory(source: &Path) -> io::Result<(tempfile::TempDir, PathBuf)> {
    let temporary = tempfile::Builder::new()
        .prefix("antlr-rust-support-")
        .tempdir()?;
    let staged = temporary.path().join("grammar");
    copy_directory(source, &staged)?;
    Ok((temporary, fs::canonicalize(staged)?))
}

pub(crate) fn prepare_support_output_directory(staged: &Path) -> io::Result<()> {
    // Bundles that ship Rust modules may copy generated support into this convention.
    fs::create_dir_all(staged.join("src"))
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

pub(crate) fn execute_transform(
    executable: &Path,
    staged: &Path,
    timeout: Duration,
) -> io::Result<()> {
    let mut child = Command::new(executable)
        .arg("--__antlr-rust-transform")
        .arg(staged)
        .current_dir(staged)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("Rust target transform stdout pipe was not available"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("Rust target transform stderr pipe was not available"))?;
    let stdout_reader = thread::spawn(move || read_capped(stdout));
    let stderr_reader = thread::spawn(move || read_capped(stderr));
    let started = Instant::now();
    let (status, timed_out) = loop {
        if let Some(status) = child.try_wait()? {
            break (status, false);
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            break (child.wait()?, true);
        }
        #[allow(clippy::disallowed_methods)] // Synchronous child polling has no async runtime.
        thread::sleep(Duration::from_millis(20));
    };
    let stdout = join_reader(stdout_reader, "stdout")?;
    let stderr = join_reader(stderr_reader, "stderr")?;
    let stderr = diagnostic_output("stderr", &stderr);
    let stdout = diagnostic_output("stdout", &stdout);

    if timed_out {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!(
                "Rust target transform exceeded its {} second deadline{stderr}{stdout}",
                timeout.as_secs()
            ),
        ));
    }
    if status.success() {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "Rust target transform failed with {status}{stderr}{stdout}"
    )))
}

fn read_capped(mut reader: impl io::Read) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut chunk = [0; 8192];
    loop {
        let count = reader.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        let retained = count.min(OUTPUT_LIMIT.saturating_sub(output.len()));
        output.extend_from_slice(&chunk[..retained]);
    }
    Ok(output)
}

fn join_reader(
    reader: thread::JoinHandle<io::Result<Vec<u8>>>,
    stream: &str,
) -> io::Result<Vec<u8>> {
    reader
        .join()
        .map_err(|_| io::Error::other(format!("Rust target transform {stream} reader panicked")))?
}

fn diagnostic_output(stream: &str, bytes: &[u8]) -> String {
    let output = String::from_utf8_lossy(bytes);
    let output = output.trim();
    if output.is_empty() {
        String::new()
    } else {
        format!("\n{stream}:\n{output}")
    }
}

pub(crate) fn top_level_files_with_extension(
    directory: &Path,
    extension: &str,
) -> io::Result<Vec<PathBuf>> {
    let mut files = fs::read_dir(directory)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<io::Result<Vec<_>>>()?;
    files.retain(|path| path.is_file() && path.extension() == Some(OsStr::new(extension)));
    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{OUTPUT_LIMIT, read_capped};

    #[test]
    fn capped_reader_drains_without_retaining_unbounded_output() {
        let input = vec![b'x'; OUTPUT_LIMIT + 1024];
        let output = read_capped(Cursor::new(input)).expect("reader should succeed");
        assert_eq!(output.len(), OUTPUT_LIMIT);
    }
}
