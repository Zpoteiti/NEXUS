/// MIME type detection utilities.
///
/// Two complementary strategies:
/// - `detect_mime_from_extension` — fast, filename-based lookup
/// - `detect_mime_from_bytes` — magic-byte sniffing for image formats
///
/// No filesystem access; callers supply filenames or already-read bytes.

/// Detect MIME type from a filename extension.
///
/// Covers common image, document, audio, and video types.
pub fn detect_mime_from_extension(filename: &str) -> Option<&'static str> {
    let lower = filename.to_lowercase();
    if lower.ends_with(".png") {
        Some("image/png")
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        Some("image/jpeg")
    } else if lower.ends_with(".gif") {
        Some("image/gif")
    } else if lower.ends_with(".webp") {
        Some("image/webp")
    } else if lower.ends_with(".bmp") {
        Some("image/bmp")
    } else if lower.ends_with(".pdf") {
        Some("application/pdf")
    } else if lower.ends_with(".txt") {
        Some("text/plain")
    } else if lower.ends_with(".json") {
        Some("application/json")
    } else if lower.ends_with(".csv") {
        Some("text/csv")
    } else if lower.ends_with(".zip") {
        Some("application/zip")
    } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        Some("application/gzip")
    } else if lower.ends_with(".mp3") {
        Some("audio/mpeg")
    } else if lower.ends_with(".mp4") {
        Some("video/mp4")
    } else {
        None
    }
}

/// Detect image MIME type from raw file bytes (magic-byte sniffing).
///
/// Returns `None` for non-image or unrecognised formats.
pub fn detect_mime_from_bytes(data: &[u8]) -> Option<&'static str> {
    if data.len() >= 8 && data[0..8] == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] {
        return Some("image/png");
    }
    if data.len() >= 3 && data[0..3] == [0xFF, 0xD8, 0xFF] {
        return Some("image/jpeg");
    }
    if data.len() >= 6 {
        match &data[0..6] {
            b"GIF87a" | b"GIF89a" => return Some("image/gif"),
            _ => {}
        }
    }
    if data.len() >= 12 && data[0..4] == *b"RIFF" && &data[8..12] == *b"WEBP" {
        return Some("image/webp");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── extension-based ──────────────────────────────────────────────

    #[test]
    fn extension_png() {
        assert_eq!(detect_mime_from_extension("photo.PNG"), Some("image/png"));
    }

    #[test]
    fn extension_jpeg() {
        assert_eq!(detect_mime_from_extension("a.jpg"), Some("image/jpeg"));
        assert_eq!(detect_mime_from_extension("a.JPEG"), Some("image/jpeg"));
    }

    #[test]
    fn extension_gif() {
        assert_eq!(detect_mime_from_extension("x.gif"), Some("image/gif"));
    }

    #[test]
    fn extension_webp() {
        assert_eq!(detect_mime_from_extension("x.webp"), Some("image/webp"));
    }

    #[test]
    fn extension_bmp() {
        assert_eq!(detect_mime_from_extension("x.bmp"), Some("image/bmp"));
    }

    #[test]
    fn extension_pdf() {
        assert_eq!(detect_mime_from_extension("doc.pdf"), Some("application/pdf"));
    }

    #[test]
    fn extension_unknown() {
        assert_eq!(detect_mime_from_extension("file.xyz"), None);
    }

    // ── magic-byte-based ─────────────────────────────────────────────

    #[test]
    fn bytes_png() {
        let h = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00];
        assert_eq!(detect_mime_from_bytes(&h), Some("image/png"));
    }

    #[test]
    fn bytes_jpeg() {
        let h = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        assert_eq!(detect_mime_from_bytes(&h), Some("image/jpeg"));
    }

    #[test]
    fn bytes_gif() {
        assert_eq!(detect_mime_from_bytes(b"GIF89a"), Some("image/gif"));
    }

    #[test]
    fn bytes_webp() {
        assert_eq!(detect_mime_from_bytes(b"RIFF\x00\x00\x00\x00WEBP"), Some("image/webp"));
    }

    #[test]
    fn bytes_non_image() {
        assert_eq!(detect_mime_from_bytes(b"hello world"), None);
    }
}
