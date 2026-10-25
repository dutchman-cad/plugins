// SPDX-License-Identifier: Apache-2.0
// Adapted from Dutchman; see ATTRIBUTION.md.
//! The one place Dutchman calls cadmpeg's decoders.

use std::io::Cursor;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use cadmpeg_core::CodecError;
use cadmpeg_core::decode::{DecodeArena, DecodeContext, DecodeDepth, DecodePolicy, ResourceLimits};
use cadmpeg_ir::codec::{Codec, DecodeFailure, DecodeOptions};
use cadmpeg_ir::report::Severity as CadmpegSeverity;
use cadmpeg_ir::validate::validate_neutral_for_decode;

use crate::{Error, FILE_DID_NOT_CONVERT, Format, MAX_INPUT_BYTES, Model, Report, Severity, Stage};

/// cadmpeg's desktop ceilings in salvage mode: a successful decode keeps going past a loss
/// and reports it, rather than refusing the whole file. The input cap is Dutchman's own.
fn policy() -> DecodePolicy {
    DecodePolicy {
        limits: ResourceLimits {
            max_input_bytes: MAX_INPUT_BYTES,
            ..ResourceLimits::desktop()
        },
        ..DecodePolicy::desktop()
    }
}

/// Decodes `bytes`, the contents of the `format` file at `path`, into the neutral model.
///
/// cadmpeg is third-party code run on hostile input, so a panic inside it is caught and
/// returned as [`Error::Decode`] instead of taking the process down.
pub(crate) fn decode<C: Codec>(
    codec: C,
    format: Format,
    path: &Path,
    bytes: &[u8],
    report: &mut Report,
) -> Result<Model, Error> {
    catch_unwind(AssertUnwindSafe(|| {
        let model = decode_unguarded(codec, format, path, bytes, report).map_err(|error| match error {
            Failed::Checks(why) => Error::Decode(why),
            Failed::Other(error) => error,
        })?;
        report.push_at(
            Stage::Project,
            Severity::Degraded,
            "history",
            "Geometry-only conversion: editable feature history, sketches and parameters are not imported",
        );
        Ok(model)
    }))
    .unwrap_or_else(|_| {
        Err(Error::Decode(format!(
            "the {} decoder failed unexpectedly on this file",
            format.name()
        )))
    })
}

/// Why a decode at one depth gave no model.
#[derive(Debug)]
enum Failed {
    /// cadmpeg decoded the file, but its model fails cadmpeg's checks: why.
    Checks(String),
    /// Anything else.
    Other(Error),
}

/// How a failure of cadmpeg's checks begins.
const CHECKS: &str = "cadmpeg's model of it fails its checks ";

fn decode_unguarded<C: Codec>(
    codec: C,
    format: Format,
    path: &Path,
    bytes: &[u8],
    report: &mut Report,
) -> Result<Model, Failed> {
    let other = |err: Error| Failed::Other(err);
    let policy = policy();
    // Static dispatch keeps unused detect/inspect methods out of the component.
    let options = DecodeOptions {
        depth: DecodeDepth::Geometry,
        policy,
    };
    let mut input = Cursor::new(bytes);
    let decoded = codec
        .decode(&mut input, &options)
        .map_err(|err| decode_failure(format, err))
        .map_err(other)?;
    let (ir, decode_report, fidelity) = decoded.into_parts();
    // External assembly part files are unavailable through this stream-only adapter.
    if !decode_report.geometry_transferred() {
        return Err(other(Error::Decode(format!(
            "no geometry could be read from this {} file",
            format.name()
        ))));
    }
    report.extend_from_cadmpeg(&decode_report.losses);

    // Check the model the way cadmpeg's own conversion does before anything is built on it.
    let arena = DecodeArena::new();
    let decode_error = |err: &dyn std::fmt::Display| other(Error::Decode(err.to_string()));
    let ctx = DecodeContext::for_loaded_input(&arena, &policy, bytes.len() as u64)
        .map_err(|err| decode_error(&err))?;
    let validation =
        validate_neutral_for_decode(&ctx, &ir, Vec::new()).map_err(|err| decode_error(&err))?;
    ctx.finish_session().map_err(|err| decode_error(&err))?;
    if !validation.is_ok() {
        let first = validation
            .findings
            .iter()
            .find(|finding| finding.severity >= CadmpegSeverity::Error)
            .map(|finding| format!(": {}", finding.message))
            .unwrap_or_default();
        return Err(Failed::Checks(format!(
            "{CHECKS}({} errors){first}",
            validation.error_count()
        )));
    }
    for finding in &validation.findings {
        report.push_at(
            Stage::Project,
            finding.severity.into(),
            "validation",
            finding.message.clone(),
        );
    }

    Ok(Model {
        ir,
        fidelity: Some(fidelity),
        name: design_name(path),
        source_file: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned()),
        assembly: None,
    })
}

/// Keep cadmpeg's cause, and give a recovery step for unreadable containers.
fn decode_failure(format: Format, err: DecodeFailure) -> Error {
    let recovery = matches!(
        &err,
        DecodeFailure::Codec(
            CodecError::Malformed(_) | CodecError::WrongFormat(_) | CodecError::Truncated { .. }
        )
    );
    let cause = match err {
        DecodeFailure::Codec(err) => err.to_string(),
        other => other.to_string(),
    };
    if recovery {
        Error::Decode(format!(
            "{cause}. The file may be incomplete or not a {} file; download or copy it again, \
             or check its extension. If the original opens in {}, report it with the file's \
             anonymous report: {FILE_DID_NOT_CONVERT}",
            format.name(),
            format.family()
        ))
    } else {
        Error::Decode(cause)
    }
}

/// The design's name: the input file's stem, or `Design` when it has none.
fn design_name(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .filter(|stem| !stem.trim().is_empty())
        .unwrap_or_else(|| "Design".to_owned())
}
