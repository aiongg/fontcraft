//! The CID-keyed parameters of `font.compile` (M12.41; `fontcraft_compile::cid`): the CMap files
//! named by `cidCmap` and `cidMacCmap`, read here, and the Macintosh `cmap` ids. The compile
//! crate parses the files and uses them only when the font is CID-keyed. The Unicode Variation
//! Sequences file `uvsFile` is read here too and serves every font.

use fontcraft_compile::Options;
use fontcraft_compile::cid::{CidOptions, MAX_CMAP_BYTES};

use crate::Result;
use crate::params::Params;

/// Fills `options.cid` from the parameters.
pub(super) fn read(p: &Params, options: &mut Options) -> Result<()> {
    let file = |key: &str| -> Result<Option<Vec<u8>>> {
        let Some(path) = p.opt_str(key)? else { return Ok(None) };
        let input = super::file_io::read_input_up_to(std::path::Path::new(path), MAX_CMAP_BYTES)?;
        if input.bytes.len() > MAX_CMAP_BYTES {
            return Err(p.bad(format!("`{key}`: {path} is larger than {MAX_CMAP_BYTES} bytes")));
        }
        Ok(Some(input.bytes))
    };
    let id = |key: &str| -> Result<Option<u16>> { Ok(p.opt_int(key, 0, 32_767)?.and_then(|v| u16::try_from(v).ok())) };
    options.cid = CidOptions {
        unicode_cmap: file("cidCmap")?,
        mac_cmap: file("cidMacCmap")?,
        mac_script: id("cidMacScript")?,
        mac_language: id("cidMacLanguage")?,
        uvs: file("uvsFile")?,
    };
    Ok(())
}
