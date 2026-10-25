// SPDX-License-Identifier: MIT OR Apache-2.0
wit_bindgen::generate!({ path: "../../wit", world: "importer-plugin", features: ["file-formats"], pub_export_macro: true });
mod container;
mod decode;
mod document;
mod step;
pub use dutchman_core::{
    Error, FILE_DID_NOT_CONVERT, Format, MAX_INPUT_BYTES, Model, Report, Severity, Stage,
};
use exports::solidcraft::plugin::importer::Guest;
use solidcraft::plugin::{input, output, types::PluginError};
use std::path::Path;
/// A single format family supplied by a component, with statically dispatched decoding.
pub trait Family {
    type Codec: cadmpeg_ir::codec::Codec;
    fn codec() -> Self::Codec;
    fn format(extension: &str) -> Option<Format>;
}
/// Shared stream adapter; each leaf component exports one concrete family.
pub struct Importer<F>(std::marker::PhantomData<F>);
struct Stream;
impl std::io::Write for Stream {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        // Leave room for canonical ABI argument overhead under the host's 8 MiB copy cap.
        for chunk in bytes.chunks(1024 * 1024) {
            output::write(chunk).map_err(|e| std::io::Error::other(format!("{e:?}")))?;
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl<F: Family> Guest for Importer<F> {
    fn import_design(main: input::FileInfo) -> Result<String, PluginError> {
        let extension = Path::new(&main.name)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let format = F::format(&extension).ok_or_else(|| {
            PluginError::Invalid("Extension not supported by this importer".into())
        })?;
        if main.size > MAX_INPUT_BYTES {
            return Err(PluginError::Limit("Input exceeds 1 GiB".into()));
        }
        let mut bytes = Vec::new();
        let mut offset = 0;
        while offset < main.size {
            let len = (main.size - offset).min(1024 * 1024) as u32;
            let chunk = input::read("", offset, len)?;
            if chunk.is_empty() {
                return Err(PluginError::Failed("Input ended early".into()));
            }
            offset += chunk.len() as u64;
            bytes.extend_from_slice(&chunk);
        }
        let mut report = Report::default();
        let mut model = decode::decode(
            F::codec(),
            format,
            Path::new(&main.name),
            &bytes,
            &mut report,
        )
        .map_err(|e| PluginError::Failed(e.to_string()))?;
        if format == Format::FreeCadDocument {
            let visible = model
                .ir
                .model
                .bodies
                .iter()
                .filter(|b| b.visible != Some(false))
                .count();
            let hidden = model.ir.model.bodies.len().saturating_sub(visible);
            if hidden > 0 && visible > 0 {
                model.ir.model.bodies.retain(|b| b.visible != Some(false));
                let ids: std::collections::BTreeSet<_> =
                    model.ir.model.bodies.iter().map(|b| b.id.clone()).collect();
                for product in &mut model.ir.model.product_definitions {
                    product.bodies.retain(|id| ids.contains(id));
                }
                report.push(
                    Severity::Info,
                    "bodies",
                    format!("{hidden} hidden intermediate shapes omitted"),
                );
            }
        }
        let step =
            step::restate(&model, &mut report).map_err(|e| PluginError::Failed(e.to_string()))?;
        let container = container::Container::new(step.as_bytes())
            .map_err(|e| PluginError::Failed(e.to_string()))?;
        let document = document::import_design(&model, container.step_blob());
        document::report_unmapped(&model, &mut report);
        let bytes = container
            .finish(&document)
            .map_err(|e| PluginError::Failed(e.to_string()))?;
        std::io::Write::write_all(&mut Stream, &bytes)
            .map_err(|e| PluginError::Failed(e.to_string()))?;
        let text = report
            .losses()
            .iter()
            .map(|loss| format!("{loss:?}\n"))
            .collect::<String>();
        if text.len() > 65536 {
            return Err(PluginError::Limit("Loss report exceeds 64 KiB".into()));
        }
        Ok(if text.is_empty() {
            "Converted without reported losses.".into()
        } else {
            text
        })
    }
}
