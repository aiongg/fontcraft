//! zlib (WOFF 1) and Brotli (WOFF2), with decompression capped at the size the file states.

use std::io::Read;

use brotli::enc::StandardAlloc;
use brotli::enc::backward_references::BrotliEncoderMode;
use brotli::enc::encode::{BrotliEncoderOperation, BrotliEncoderParameter, BrotliEncoderStateStruct};
use brotli::enc::input_pair::InputReferenceMut;
use brotli::enc::interface::{InputPair, PredictionModeContextMap, StaticCommand};

use crate::Error;

/// fontTools' zlib level for WOFF 1 tables and metadata (`ZLIB_COMPRESSION_LEVEL`).
pub const ZLIB_LEVEL: u8 = 6;
/// The Brotli settings of fontTools' WOFF2 writer (the Brotli library's defaults).
pub const BROTLI_QUALITY: u32 = 11;
pub const BROTLI_WINDOW: u32 = 22;

/// Brotli's compression mode hint.
#[derive(Debug, Clone, Copy)]
pub enum Mode {
    /// The table data of a WOFF2 file.
    Font,
    /// The metadata block of a WOFF2 file.
    Text,
}

pub fn zlib(data: &[u8]) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec_zlib(data, ZLIB_LEVEL)
}

/// Inflates a zlib stream that must produce exactly `size` bytes; stops past `size`.
pub fn unzlib(format: &'static str, data: &[u8], size: usize, what: &str) -> Result<Vec<u8>, Error> {
    let out = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(data, size)
        .map_err(|_| Error::invalid(format, format!("{what} does not inflate to its stated size")))?;
    if out.len() != size {
        return Err(Error::invalid(format, format!("{what} inflates to another size than stated")));
    }
    Ok(out)
}

fn no_callback(_: &mut PredictionModeContextMap<InputReferenceMut<'_>>, _: &mut [StaticCommand], _: InputPair<'_>, _: &mut StandardAlloc) {}

/// Most rounds of the encoder loop: each round empties a 64 KiB output buffer, so this bounds
/// the output at 16 GiB, far above any font this crate accepts.
const MAX_ROUNDS: usize = 1 << 18;

/// Compresses `data` in one Brotli stream, handing the encoder all of it at once with the
/// "finish" operation, as the Python Brotli binding fontTools uses does.
pub fn brotli(data: &[u8], mode: Mode) -> Result<Vec<u8>, Error> {
    let mut state = BrotliEncoderStateStruct::new(StandardAlloc::default());
    let mode = match mode {
        Mode::Font => BrotliEncoderMode::BROTLI_MODE_FONT,
        Mode::Text => BrotliEncoderMode::BROTLI_MODE_TEXT,
    };
    let set = state.set_parameter(BrotliEncoderParameter::BROTLI_PARAM_MODE, mode as u32)
        && state.set_parameter(BrotliEncoderParameter::BROTLI_PARAM_QUALITY, BROTLI_QUALITY)
        && state.set_parameter(BrotliEncoderParameter::BROTLI_PARAM_LGWIN, BROTLI_WINDOW)
        && state.set_parameter(BrotliEncoderParameter::BROTLI_PARAM_LGBLOCK, 0);
    let failed = || Error::invalid("WOFF2", "the Brotli encoder failed");
    if !set {
        return Err(failed());
    }
    let mut buffer = vec![0u8; 1 << 16];
    let mut out = Vec::new();
    let mut available_in = data.len();
    let mut in_offset = 0;
    for _ in 0..MAX_ROUNDS {
        let mut available_out = buffer.len();
        let mut out_offset = 0;
        let mut total = None;
        let ok = state.compress_stream(
            BrotliEncoderOperation::BROTLI_OPERATION_FINISH,
            &mut available_in,
            data,
            &mut in_offset,
            &mut available_out,
            &mut buffer,
            &mut out_offset,
            &mut total,
            &mut no_callback,
        );
        if !ok {
            return Err(failed());
        }
        out.extend_from_slice(buffer.get(..out_offset).ok_or_else(failed)?);
        if state.is_finished() && !state.has_more_output() {
            return Ok(out);
        }
    }
    Err(failed())
}

/// Decompresses a Brotli stream that must produce exactly `size` bytes; stops past `size`.
pub fn unbrotli(format: &'static str, data: &[u8], size: usize, what: &str) -> Result<Vec<u8>, Error> {
    let mut out = Vec::with_capacity(size.min(1 << 20));
    let limit = u64::try_from(size).unwrap_or(u64::MAX).saturating_add(1);
    brotli::Decompressor::new(data, 1 << 12)
        .take(limit)
        .read_to_end(&mut out)
        .map_err(|_| Error::invalid(format, format!("{what} does not decompress")))?;
    if out.len() != size {
        return Err(Error::invalid(format, format!("{what} decompresses to another size than stated")));
    }
    Ok(out)
}
