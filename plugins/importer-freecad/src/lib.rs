// SPDX-License-Identifier: MIT OR Apache-2.0
use dutchman_importer_common::{Family, Format};
struct Decoder;
impl Family for Decoder {
    type Codec = cadmpeg_codec_freecad::FcstdCodec;
    fn codec() -> Self::Codec {
        cadmpeg_codec_freecad::FcstdCodec
    }
    fn format(extension: &str) -> Option<Format> {
        match extension {
            "fcstd" => Some(Format::FreeCadDocument),
            _ => None,
        }
    }
}
type Importer = dutchman_importer_common::Importer<Decoder>;
// Only generated canonical ABI trampolines require unsafe code.
#[allow(unsafe_code)]
mod component_exports {
    use super::Importer;
    dutchman_importer_common::export!(Importer with_types_in dutchman_importer_common);
}
