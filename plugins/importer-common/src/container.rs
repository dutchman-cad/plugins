// SPDX-License-Identifier: Apache-2.0
// From Dutchman; see ATTRIBUTION.md.
//! SolidCraft's design container: a zip holding `document.json`, `container.json` and
//! content-addressed payloads under `blobs/`, laid out as SolidCraft writes it.

use std::io::{Cursor, Write};

use dutchman_core::Error;
use serde_json::Value;
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// The container version SolidCraft writes and reads.
const VERSION: &str = "solidcraft-container/1";

/// A container being written: payloads first, then the document that refers to them.
pub(crate) struct Container {
    zip: ZipWriter<Cursor<Vec<u8>>>,
    step_blob: String,
    /// Blob paths already written, so an identical payload is stored once.
    blobs: Vec<String>,
}

impl Container {
    /// Starts a container holding `step` as its one STEP payload.
    pub(crate) fn new(step: &[u8]) -> Result<Self, Error> {
        let mut container = Container::empty();
        container.step_blob = container.add_step(step)?;
        Ok(container)
    }

    /// Starts a container with no payloads yet.
    pub(crate) fn empty() -> Self {
        Container {
            zip: ZipWriter::new(Cursor::new(Vec::new())),
            step_blob: String::new(),
            blobs: Vec::new(),
        }
    }

    /// Adds another STEP payload, stored once by its content, and returns its blob path.
    pub(crate) fn add_step(&mut self, step: &[u8]) -> Result<String, Error> {
        let name = format!("blobs/{}.step", hex(&Sha256::digest(step)));
        if !self.blobs.contains(&name) {
            self.part(&name, step, CompressionMethod::Stored)?;
            self.blobs.push(name.clone());
        }
        Ok(name)
    }

    /// The container path of the STEP payload, as the document refers to it.
    pub(crate) fn step_blob(&self) -> &str {
        &self.step_blob
    }

    /// Adds `document` and the container stamp, and returns the finished container.
    pub(crate) fn finish(mut self, document: &Value) -> Result<Vec<u8>, Error> {
        let document = serde_json::to_vec(document).map_err(encode)?;
        self.part("document.json", &document, CompressionMethod::Deflated)?;
        let stamp =
            serde_json::to_vec(&serde_json::json!({ "format": VERSION })).map_err(encode)?;
        self.part("container.json", &stamp, CompressionMethod::Stored)?;
        Ok(self.zip.finish().map_err(encode)?.into_inner())
    }

    fn part(&mut self, name: &str, bytes: &[u8], method: CompressionMethod) -> Result<(), Error> {
        let level = (method == CompressionMethod::Deflated).then_some(1);
        let options = SimpleFileOptions::default()
            .compression_method(method)
            .compression_level(level);
        self.zip.start_file(name, options).map_err(encode)?;
        self.zip.write_all(bytes).map_err(encode)
    }
}

/// Lower-case hex, as SolidCraft names its blobs.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn encode(err: impl std::fmt::Display) -> Error {
    Error::Encode(format!("cannot build the design container: {err}"))
}
