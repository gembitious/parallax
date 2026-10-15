//! 파일 내용 디코드. 화면에 보여 줄 문자열로 바꾼다.
//!
//! UTF-8이 기본이고, UTF-8이 아니면 EUC-KR(CP949)을 시도한다(오래된 한국어 소스에 흔하다).
//! 줄 구분은 바이트 `\n` 그대로라 git blame의 줄 번호와 어긋나지 않는다.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Encoding {
    #[serde(rename = "UTF-8")]
    Utf8,
    /// EUC-KR과 그 확장인 CP949(windows-949)
    #[serde(rename = "EUC-KR")]
    EucKr,
    /// 어느 쪽으로도 깨끗하게 읽히지 않아 UTF-8로 읽고 깨진 바이트를 U+FFFD로 바꿨다.
    #[serde(rename = "UTF-8 (깨진 바이트)")]
    Lossy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Decoded {
    pub text: String,
    pub encoding: Encoding,
    pub line_count: u32,
}

/// 바이너리로 보는 기준: 앞 8000바이트 안에 NUL이 있다(git과 같은 휴리스틱).
pub fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8000).any(|&b| b == 0)
}

pub fn decode(bytes: &[u8]) -> Decoded {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let line_count = line_count(bytes);
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Decoded {
            text: text.to_owned(),
            encoding: Encoding::Utf8,
            line_count,
        };
    }
    let (text, _, had_errors) = encoding_rs::EUC_KR.decode(bytes);
    if !had_errors {
        return Decoded {
            text: text.into_owned(),
            encoding: Encoding::EucKr,
            line_count,
        };
    }
    Decoded {
        text: String::from_utf8_lossy(bytes).into_owned(),
        encoding: Encoding::Lossy,
        line_count,
    }
}

/// git blame과 같은 방식으로 센 줄 수: `\n`마다 한 줄, 끝에 개행이 없으면 마지막 조각도 한 줄.
pub fn line_count(bytes: &[u8]) -> u32 {
    let newlines = bytes.iter().filter(|&&b| b == b'\n').count() as u32;
    newlines + u32::from(!bytes.is_empty() && !bytes.ends_with(b"\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_and_bom() {
        let d = decode("가\n나\n".as_bytes());
        assert_eq!(
            (d.text.as_str(), d.encoding, d.line_count),
            ("가\n나\n", Encoding::Utf8, 2)
        );
        let d = decode(b"\xef\xbb\xbfa");
        assert_eq!((d.text.as_str(), d.line_count), ("a", 1));
    }

    #[test]
    fn euc_kr() {
        let d = decode(b"\xbe\xc8\xb3\xe7\xc7\xcf\xbc\xbc\xbf\xe4\n");
        assert_eq!(
            (d.text.as_str(), d.encoding),
            ("안녕하세요\n", Encoding::EucKr)
        );
    }

    #[test]
    fn lossy() {
        // EUC-KR로도 안 되는 바이트열(단독 0x80)
        let d = decode(b"a\x80\xff\n");
        assert_eq!(d.encoding, Encoding::Lossy);
        assert!(d.text.contains('\u{fffd}'));
    }

    #[test]
    fn line_counts_match_git() {
        assert_eq!(line_count(b""), 0);
        assert_eq!(line_count(b"a"), 1);
        assert_eq!(line_count(b"a\n"), 1);
        assert_eq!(line_count(b"a\r\nb"), 2);
    }

    #[test]
    fn binary_detection() {
        assert!(is_binary(b"\x89PNG\r\n\x1a\n\x00\x00"));
        assert!(!is_binary("한글".as_bytes()));
    }
}
