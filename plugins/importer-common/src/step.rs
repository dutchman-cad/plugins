// SPDX-License-Identifier: Apache-2.0
// Adapted from Dutchman; see ATTRIBUTION.md.
//! The model's geometry restated as STEP text, the form a SolidCraft Import keeps.

use cadmpeg_ir::codec::write::target::TargetRequest;
use cadmpeg_ir::codec::write::{EncodeInput, Encoder};
use dutchman_core::{Error, Model, Report, Severity};

/// The STEP schema written: AP214, which SolidCraft writes and reads natively.
const SCHEMA: &str = "ap214";

/// The name cadmpeg's STEP writer gives a product it has no name for.
const PLACEHOLDER: &str = "'cadmpeg_model'";

/// cadmpeg's code for procedural surfaces its STEP writer had no form for.
const PROCEDURAL_NOT_WRITTEN: &str = "step/geometry.procedural-definition-not-written";

/// The largest STEP text SolidCraft's reader accepts.
const MAX_STEP_BYTES: usize = 512 << 20;

/// Restates the model as STEP AP214 text, recording what the restatement drops.
///
/// cadmpeg is third-party code run on models decoded from hostile input, so a panic inside it
/// is caught and returned as [`Error::Encode`] instead of taking the process down.
pub(crate) fn restate(model: &Model, report: &mut Report) -> Result<String, Error> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        restate_unguarded(model, report)
    }))
    .unwrap_or_else(|_| {
        Err(Error::Restate(
            "cadmpeg's STEP writer failed unexpectedly on this model".to_owned(),
        ))
    })
}

/// cadmpeg's plan for restating the model as STEP, refused when it carries no geometry.
fn plan(model: &Model) -> Result<cadmpeg_ir::codec::write::ExportPlan, Error> {
    // With no bodies at all, cadmpeg plans an empty file rather than saying so.
    if model.ir.model.bodies.is_empty() {
        return Err(no_geometry());
    }
    let input = EncodeInput::new(&model.ir, model.fidelity.as_ref());
    let plan = cadmpeg_codec_step::StepCodec::default()
        .plan(input, TargetRequest::Explicit(SCHEMA))
        .map_err(|err| Error::Restate(format!("cannot restate the model as STEP: {err}")))?;
    if plan
        .report()
        .losses
        .iter()
        .any(|loss| loss.code.to_string() == "step/export.no-exportable-solids")
    {
        return Err(no_geometry());
    }
    Ok(plan)
}

fn no_geometry() -> Error {
    Error::NoGeometry("the model has no solids or surfaces SolidCraft can import".to_owned())
}

fn restate_unguarded(model: &Model, report: &mut Report) -> Result<String, Error> {
    let plan = plan(model)?;
    report.extend_from_cadmpeg_export(&plan.report().losses);
    if plan
        .report()
        .losses
        .iter()
        .any(|loss| loss.code.to_string() == PROCEDURAL_NOT_WRITTEN)
        && let Some(detail) = unwritten_surfaces(model)
    {
        report.push(Severity::Degraded, "geometry", detail);
    }

    let mut bytes = Vec::new();
    plan.write_to(&mut bytes)
        .map_err(|err| Error::Restate(format!("cannot write STEP: {err}")))?;
    if bytes.len() > MAX_STEP_BYTES {
        return Err(Error::Encode(format!(
            "the STEP restatement is {} MB, over SolidCraft's {} MB import limit",
            bytes.len() >> 20,
            MAX_STEP_BYTES >> 20
        )));
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| Error::Restate("cadmpeg wrote STEP that is not UTF-8".to_owned()))?;
    Ok(name_unnamed_product(text, &model.name))
}

/// What the procedural surfaces cadmpeg could not restate are, said plainly: a coil's helix
/// surface is left out of the STEP, so the solid it bounds opens with no volume.
fn unwritten_surfaces(model: &Model) -> Option<String> {
    let helices = model
        .ir
        .model
        .procedural_surfaces
        .iter()
        .filter(|surface| {
            matches!(
                surface.definition(),
                cadmpeg_ir::geometry::ProceduralSurfaceDefinition::Helix { .. }
            )
        })
        .count();
    match helices {
        0 => None,
        1 => Some(
            "a helix surface (the swept face of a coil or thread) has no form cadmpeg writes \
             to STEP and is left out: the solid it bounds opens without it, with no volume"
                .to_owned(),
        ),
        n => Some(format!(
            "{n} helix surfaces (the swept faces of coils or threads) have no form cadmpeg \
             writes to STEP and are left out: the solids they bound open without them, with \
             no volume"
        )),
    }
}

/// Names the product cadmpeg left unnamed after the design, as SolidCraft does for files it
/// restates through cadmpeg. The name is reduced to plain STEP string characters.
fn name_unnamed_product(step: String, design: &str) -> String {
    let name: String = design
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'))
        .collect();
    if name.trim().is_empty() || !step.contains(PLACEHOLDER) {
        step
    } else {
        step.replace(PLACEHOLDER, &format!("'{name}'"))
    }
}
