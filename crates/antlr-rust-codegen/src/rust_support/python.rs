use std::path::Path;

use miette::{IntoDiagnostic as _, WrapErr as _};
use rustpython_vm::common::rc::PyRc;
use rustpython_vm::{Interpreter, Settings};

const TRANSFORM_PATH: &str = "Rust/transformGrammar.py";

pub(crate) fn run_transform_child(staging_directory: &Path) -> miette::Result<()> {
    let staging_directory = std::fs::canonicalize(staging_directory)
        .into_diagnostic()
        .wrap_err("failed to open the staged grammar directory")?;
    let script = staging_directory.join(TRANSFORM_PATH);
    if !script.is_file() {
        return Err(miette::miette!(
            "staged Rust target transform does not exist: {}",
            script.display()
        ));
    }
    std::env::set_current_dir(&staging_directory)
        .into_diagnostic()
        .wrap_err("failed to enter the staged grammar directory")?;
    let script = script.to_str().ok_or_else(|| {
        miette::miette!(
            "staged Rust target transform path is not UTF-8: {}",
            script.display()
        )
    })?;

    let mut settings = Settings::default();
    settings.argv = vec![script.to_owned()];
    settings.ignore_environment = true;
    settings.import_site = false;
    settings.install_signal_handlers = false;
    settings.isolated = true;
    settings.quiet = true;
    settings.safe_path = true;
    settings.user_site_directory = false;
    settings.write_bytecode = false;

    let builder = Interpreter::builder(settings);
    let interpreter = builder
        .add_frozen_modules(rustpython_pylib::FROZEN_STDLIB)
        .init_hook(|vm| {
            let state = PyRc::get_mut(&mut vm.state)
                .expect("RustPython global state is uniquely owned during initialization");
            state.config.paths.stdlib_dir = Some(rustpython_pylib::LIB_PATH.to_owned());
        })
        .build();
    let exit_code = interpreter.run(|vm| {
        let scope = vm.new_scope_with_main()?;
        vm.run_script(scope, script)
    });
    if exit_code == 0 {
        Ok(())
    } else {
        Err(miette::miette!(
            "Rust target transform exited with status {exit_code}"
        ))
    }
}
