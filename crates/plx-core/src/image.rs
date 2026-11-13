//! 그림 파일 알아보기(이미지 diff). 바뀐 파일의 두 버전이 그림이면 커밋 상세가 줄 diff 대신 전·후 그림을 보인다.
//!
//! 종류는 내용의 앞 바이트(매직 넘버)로 정한다: 확장자가 틀리거나 없어도 보이고, `.png`인데 그림이 아닌 파일
//! (Git LFS 포인터 등)은 그림으로 치지 않는다. SVG는 글 파일이라 확장자(`.svg`)와 내용(`<svg` 요소)을 함께 본다.
//! 그림을 풀어 그리는 것은 WebView의 몫이고(`<img>`, SVG의 스크립트는 돌지 않는다) 여기서는 크기도 읽지 않는다.

use serde::Serialize;

/// 이보다 큰 그림(한쪽)은 보내지 않는다. 줄 diff의 한도([`crate::commit::MAX_VERSIONS_BYTES`])보다 크다:
/// 그림은 바이트를 화면에 그대로 넘기기만 하고 줄로 견주지 않는다.
pub const MAX_IMAGE_BYTES: u64 = 32 << 20;

/// 그림의 한쪽 버전
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageSide {
    /// `image/png` 같은 MIME 형식(WebView에 넘길 때 쓴다)
    pub mime: &'static str,
    pub bytes: u64,
}

/// 두 버전의 그림. 없는 쪽(추가·삭제)은 `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageVersions {
    pub old: Option<ImageSide>,
    pub new: Option<ImageSide>,
}

/// 확장자로 보아 그림일 수 있는 경로인가(큰 파일 한도를 그림 한도로 볼지 정한다)
pub fn image_path(path: &str) -> bool {
    let ext = match path.rsplit_once('.') {
        Some((_, ext)) if !ext.contains('/') => ext.to_ascii_lowercase(),
        _ => return false,
    };
    matches!(
        ext.as_str(),
        "png"
            | "apng"
            | "jpg"
            | "jpeg"
            | "jfif"
            | "gif"
            | "webp"
            | "bmp"
            | "ico"
            | "cur"
            | "avif"
            | "svg"
    )
}

/// SVG 경로인가
fn svg_path(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("svg"))
}

/// 내용의 앞 바이트로 본 그림 형식. 그림이 아니면 `None`. `path`는 SVG만 본다.
pub fn sniff(path: &str, bytes: &[u8]) -> Option<&'static str> {
    const RASTER: &[(&[u8], &str)] = &[
        (b"\x89PNG\r\n\x1a\n", "image/png"),
        (b"\xff\xd8\xff", "image/jpeg"),
        (b"GIF87a", "image/gif"),
        (b"GIF89a", "image/gif"),
        (b"BM", "image/bmp"),
        (b"\x00\x00\x01\x00", "image/x-icon"),
        (b"\x00\x00\x02\x00", "image/x-icon"),
    ];
    if let Some((_, mime)) = RASTER.iter().find(|(magic, _)| bytes.starts_with(magic)) {
        // BMP는 "BM" 두 글자뿐이라 글 파일과 헷갈린다: 머리(14 + 정보 머리 크기)의 모양도 본다.
        if *mime == "image/bmp" && !bmp_header(bytes) {
            return None;
        }
        // ICO는 그림 수가 1 이상이고 머리 뒤에 그 수만큼 항목(16바이트)이 있어야 한다.
        if *mime == "image/x-icon" && !ico_header(bytes) {
            return None;
        }
        return Some(mime);
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    // ISO BMFF: 4바이트 크기, `ftyp`, 주 브랜드
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" && matches!(&bytes[8..12], b"avif" | b"avis") {
        return Some("image/avif");
    }
    if svg_path(path) && svg_content(bytes) {
        return Some("image/svg+xml");
    }
    None
}

fn bmp_header(bytes: &[u8]) -> bool {
    if bytes.len() < 18 {
        return false;
    }
    let info = u32::from_le_bytes([bytes[14], bytes[15], bytes[16], bytes[17]]);
    // BITMAPCOREHEADER(12) ~ BITMAPV5HEADER(124), 예약 칸(6..10)은 0
    matches!(info, 12 | 40 | 52 | 56 | 64 | 108 | 124) && bytes[6..10] == [0, 0, 0, 0]
}

fn ico_header(bytes: &[u8]) -> bool {
    if bytes.len() < 6 {
        return false;
    }
    let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    count > 0 && bytes.len() >= 6 + 16 * count.min(1)
}

/// 글 앞쪽에 `<svg` 요소가 있는가(XML 선언·주석·DOCTYPE 뒤여도 된다)
fn svg_content(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(4096)];
    let head = head.strip_prefix(b"\xef\xbb\xbf").unwrap_or(head);
    head.windows(4).any(|w| w == b"<svg")
}

/// 두 버전이 모두 그림이면(없는 쪽은 빈다) 그 형식. 한쪽만 그림이면(그림 → 글) 그림으로 치지 않는다.
pub fn versions(path: &str, old: Option<&[u8]>, new: Option<&[u8]>) -> Option<ImageVersions> {
    let side = |bytes: Option<&[u8]>| -> Option<Option<ImageSide>> {
        match bytes {
            None => Some(None),
            Some(bytes) => sniff(path, bytes).map(|mime| {
                Some(ImageSide {
                    mime,
                    bytes: bytes.len() as u64,
                })
            }),
        }
    };
    let (old, new) = (side(old)?, side(new)?);
    if old.is_none() && new.is_none() {
        return None;
    }
    Some(ImageVersions { old, new })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_raster_formats() {
        assert_eq!(
            sniff("a.bin", b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"),
            Some("image/png")
        );
        assert_eq!(
            sniff("a", b"\xff\xd8\xff\xe0\0\x10JFIF"),
            Some("image/jpeg")
        );
        assert_eq!(sniff("a.gif", b"GIF89a\x01\0\x01\0"), Some("image/gif"));
        assert_eq!(
            sniff("a.webp", b"RIFF\x24\0\0\0WEBPVP8 "),
            Some("image/webp")
        );
        assert_eq!(
            sniff("a.avif", b"\0\0\0\x1cftypavif\0\0\0\0"),
            Some("image/avif")
        );
        let mut bmp = b"BM\x46\0\0\0\0\0\0\0\x36\0\0\0\x28\0\0\0".to_vec();
        bmp.extend([0; 40]);
        assert_eq!(sniff("a.bmp", &bmp), Some("image/bmp"));
        let mut ico = b"\0\0\x01\0\x01\0".to_vec();
        ico.extend([0; 16]);
        assert_eq!(sniff("favicon.ico", &ico), Some("image/x-icon"));
    }

    #[test]
    fn text_is_not_an_image() {
        assert_eq!(
            sniff("a.png", b"version https://git-lfs.github.com/spec/v1\n"),
            None
        );
        assert_eq!(sniff("notes.txt", b"BMW drivers\n"), None);
        assert_eq!(sniff("a.ico", b"\0\0\x01\0\0\0"), None);
        assert_eq!(sniff("a.png", b""), None);
    }

    #[test]
    fn svg_needs_extension_and_element() {
        let svg =
            b"<?xml version=\"1.0\"?>\n<!-- x -->\n<svg xmlns=\"http://www.w3.org/2000/svg\"/>";
        assert_eq!(sniff("icons/a.SVG", svg), Some("image/svg+xml"));
        assert_eq!(sniff("a.xml", svg), None);
        assert_eq!(sniff("a.svg", b"<html></html>"), None);
    }

    #[test]
    fn versions_need_both_sides() {
        let png = b"\x89PNG\r\n\x1a\n".as_slice();
        assert_eq!(
            versions("a.png", None, Some(png)),
            Some(ImageVersions {
                old: None,
                new: Some(ImageSide {
                    mime: "image/png",
                    bytes: 8
                })
            })
        );
        assert_eq!(versions("a.png", Some(b"text"), Some(png)), None);
        assert_eq!(versions("a.png", None, None), None);
    }

    #[test]
    fn image_paths() {
        assert!(image_path("assets/Logo.PNG"));
        assert!(image_path("a.svg"));
        assert!(!image_path("png"));
        assert!(!image_path("a.png/readme"));
        assert!(!image_path("src/main.rs"));
    }
}
