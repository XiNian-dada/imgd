use std::path::Path;

pub fn has_webp_extension(filename: &str) -> bool {
    Path::new(filename)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.eq_ignore_ascii_case("webp"))
        .unwrap_or(false)
}

pub fn has_webp_signature(header: &[u8]) -> bool {
    header.len() >= 12 && &header[0..4] == b"RIFF" && &header[8..12] == b"WEBP"
}
