//! FontCraft engine core (L5): the parts of the engine that need no session. The engine error
//! type, command parameters ([`params`]), the user's settings ([`settings`]), the clock edits
//! are stamped with ([`clock`]), the panic guard every command runs under ([`guard`]) and the progress
//! and cancellation of a request ([`work`]).
//! `fontcraft-engine` re-exports each of them at the same path.
#![forbid(unsafe_code)]

pub mod clock;
pub mod guard;
pub mod params;
pub mod settings;
pub mod work;

pub use clock::Clock;
use fontcraft_model::Glyph;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("unknown command `{0}`")]
    UnknownCommand(String),
    #[error("the command tables are not installed")]
    NotInstalled,
    #[error("`{0}` is not available: {1}")]
    Disabled(String, String),
    #[error("{command}: {message}")]
    BadParams { command: String, message: String },
    #[error("glyph `{0}` is locked: unlock it to change it")]
    Locked(String),
    #[error("{0}")]
    Interaction(String),
    #[error("no document open")]
    NoDocument,
    #[error("cancelled")]
    Cancelled,
    #[error(transparent)]
    Model(#[from] fontcraft_model::ModelError),
    #[error(transparent)]
    Format(#[from] fontcraft_format::FormatError),
    #[error(transparent)]
    Compile(#[from] fontcraft_compile::CompileError),
    #[error(transparent)]
    Glyphs(#[from] fontcraft_io_glyphs::GlyphsError),
    #[error(transparent)]
    Ufo(#[from] fontcraft_io_ufo::UfoError),
    #[error(transparent)]
    Decompile(#[from] fontcraft_decompile::DecompileError),
    #[error(transparent)]
    Settings(#[from] settings::SettingsError),
    #[error(transparent)]
    Tables(#[from] fontcraft_tables::TablesError),
    #[error(transparent)]
    Proof(#[from] fontcraft_proof::ProofError),
    #[error("{0}")]
    Io(String),
    #[error("not supported yet: {0}")]
    Unsupported(String),
    #[error("internal error (please report it): {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, EngineError>;

/// Refuses a locked glyph (decision 0004 item 7). Every editing command goes through it, by way
/// of [`params::Targets::edit_layer`] or directly; the engine's `Session::edit` enforces the
/// same rule on whatever a command changes.
pub fn check_unlocked(glyph: &Glyph) -> Result<()> {
    if glyph.info.locked { Err(EngineError::Locked(glyph.name.clone())) } else { Ok(()) }
}
