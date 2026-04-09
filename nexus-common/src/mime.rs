//! MIME type detection by file extension and magic bytes.

pub fn detect_mime_from_extension(filename: &str) -> Option<&'static str> {
    let lower = filename.to_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        return Some("application/gzip");
    }
    let ext = lower.rsplit('.').next()?;
    match ext {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        "pdf" => Some("application/pdf"),
        "txt" => Some("text/plain"),
        "json" => Some("application/json"),
        "csv" => Some("text/csv"),
        "zip" => Some("application/zip"),
        "mp3" => Some("audio/mpeg"),
        "mp4" => Some("video/mp4"),
        _ => None,
    }
}

pub fn detect_mime_from_bytes(data: &[u8]) -> Option<&'static str> {
    if data.len() < 4 {
        return None;
    }
    match &data[..4] {
        [0x89, b'P', b'N', b'G'] => Some("image/png"),
        [0xFF, 0xD8, 0xFF, _] => Some("image/jpeg"),
        [b'G', b'I', b'F', b'8'] => Some("image/gif"),
        [b'R', b'I', b'F', b'F'] if data.len() >= 12 && &data[8..12] == b"WEBP" => {
            Some("image/webp")
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ext_png() {
        assert_eq!(detect_mime_from_extension("photo.PNG"), Some("image/png"));
    }
    #[test]
    fn ext_jpeg() {
        assert_eq!(detect_mime_from_extension("a.jpeg"), Some("image/jpeg"));
    }
    #[test]
    fn ext_gif() {
        assert_eq!(detect_mime_from_extension("x.gif"), Some("image/gif"));
    }
    #[test]
    fn ext_webp() {
        assert_eq!(detect_mime_from_extension("x.webp"), Some("image/webp"));
    }
    #[test]
    fn ext_bmp() {
        assert_eq!(detect_mime_from_extension("x.bmp"), Some("image/bmp"));
    }
    #[test]
    fn ext_pdf() {
        assert_eq!(
            detect_mime_from_extension("doc.pdf"),
            Some("application/pdf")
        );
    }
    #[test]
    fn ext_unknown() {
        assert_eq!(detect_mime_from_extension("file.xyz"), None);
    }
    #[test]
    fn ext_tar_gz() {
        assert_eq!(
            detect_mime_from_extension("a.tar.gz"),
            Some("application/gzip")
        );
    }
    #[test]
    fn bytes_png() {
        assert_eq!(
            detect_mime_from_bytes(&[0x89, b'P', b'N', b'G']),
            Some("image/png")
        );
    }
    #[test]
    fn bytes_jpeg() {
        assert_eq!(
            detect_mime_from_bytes(&[0xFF, 0xD8, 0xFF, 0xE0]),
            Some("image/jpeg")
        );
    }
    #[test]
    fn bytes_gif() {
        assert_eq!(detect_mime_from_bytes(b"GIF89a"), Some("image/gif"));
    }
    #[test]
    fn bytes_webp() {
        assert_eq!(
            detect_mime_from_bytes(b"RIFF\x00\x00\x00\x00WEBP"),
            Some("image/webp")
        );
    }
    #[test]
    fn bytes_unknown() {
        assert_eq!(detect_mime_from_bytes(b"hello world!!"), None);
    }
}
