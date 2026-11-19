//! FontCraft on the craft framework: the desktop window of `fontcraft-ui`, while it is built up
//! to replace `apps/fontcraft`.
#![forbid(unsafe_code)]

use std::process::ExitCode;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> ExitCode {
    match fontcraft_ui::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("fontcraft-next: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn main() -> ExitCode {
    ExitCode::SUCCESS
}
