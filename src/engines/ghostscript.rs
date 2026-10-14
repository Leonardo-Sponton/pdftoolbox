//! Compressão com perda via Ghostscript (`pdfwrite`), executado com `-dSAFER`.

use std::ffi::OsString;
use std::path::Path;

use super::{Tool, run_ok};
use crate::error::OpResult;
use crate::jobs::CancelToken;

/// Reescreve o PDF reamostrando imagens coloridas/cinza para `dpi` e
/// recomprimindo-as em JPEG. Detecta imagens duplicadas.
pub fn downsample(input: &Path, out: &Path, preset: &str, dpi: u32, cancel: &CancelToken) -> OpResult<()> {
    let mut output_arg = OsString::from("-sOutputFile=");
    output_arg.push(out);
    let args: Vec<OsString> = vec![
        "-q".into(),
        "-dSAFER".into(),
        "-dBATCH".into(),
        "-dNOPAUSE".into(),
        "-sDEVICE=pdfwrite".into(),
        "-dCompatibilityLevel=1.7".into(),
        format!("-dPDFSETTINGS=/{preset}").into(),
        "-dDetectDuplicateImages=true".into(),
        "-dDownsampleColorImages=true".into(),
        "-dDownsampleGrayImages=true".into(),
        "-dColorImageDownsampleType=/Bicubic".into(),
        "-dGrayImageDownsampleType=/Bicubic".into(),
        format!("-dColorImageResolution={dpi}").into(),
        format!("-dGrayImageResolution={dpi}").into(),
        format!("-dMonoImageResolution={}", dpi * 2).into(),
        "-dColorImageDownsampleThreshold=1.0".into(),
        "-dGrayImageDownsampleThreshold=1.0".into(),
        output_arg,
        // "--" não é suportado pelo gs; o caminho é absoluto, então não começa com "-".
        input.into(),
    ];
    run_ok(Tool::Ghostscript, args, &[0], cancel)?;
    Ok(())
}
