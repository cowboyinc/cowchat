//! Portable image envelope carried inside normal (including encrypted) messages.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};

pub const PREFIX: &str = "cowchat-chart:v1\n";
pub const MAX_PNG_BYTES: usize = 256 * 1024;
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Chart {
    pub title: String,
    pub png: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openui: Option<String>,
}

pub fn validate_png(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_PNG_BYTES
        || bytes.len() < 33
        || &bytes[..8] != b"\x89PNG\r\n\x1a\n"
        || bytes[8..12] != [0, 0, 0, 13]
        || &bytes[12..16] != b"IHDR"
    {
        return Err("Chart must be a PNG no larger than 256 KiB".into());
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    if width == 0
        || height == 0
        || width > 4096
        || height > 4096
        || u64::from(width) * u64::from(height) > 8_000_000
    {
        return Err("Chart dimensions must be at most 4096 × 4096 and eight million pixels".into());
    }
    Ok(())
}
impl Chart {
    #[cfg(test)]
    pub fn encode(title: &str, bytes: &[u8]) -> Result<String, String> {
        Self::encode_openui(title, bytes, None)
    }
    pub fn encode_openui(
        title: &str,
        bytes: &[u8],
        openui: Option<&str>,
    ) -> Result<String, String> {
        validate_png(bytes)?;
        if openui.is_some_and(|source| {
            source.len() > 64 * 1024
                || source
                    .chars()
                    .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        }) {
            return Err(
                "OpenUI source must be at most 64 KiB with no non-whitespace controls".into(),
            );
        }
        if title.trim().is_empty()
            || title.chars().count() > 200
            || title.chars().any(char::is_control)
        {
            return Err(
                "Chart title must contain 1–200 characters and no control characters".into(),
            );
        }
        let chart = Self {
            title: title.to_owned(),
            png: STANDARD.encode(bytes),
            openui: openui.map(String::from),
        };
        Ok(format!(
            "{PREFIX}{}",
            serde_json::to_string(&chart).map_err(|e| e.to_string())?
        ))
    }
    pub fn decode(content: &str) -> Option<(String, Vec<u8>)> {
        if content.len() > MAX_PNG_BYTES * 4 / 3 + 132 * 1024 {
            return None;
        }
        let chart: Self = serde_json::from_str(content.strip_prefix(PREFIX)?).ok()?;
        let bytes = STANDARD.decode(&chart.png).ok()?;
        // Apply exactly the same validation when reading untrusted messages.
        Self::encode_openui(&chart.title, &bytes, chart.openui.as_deref()).ok()?;
        Some((chart.title, bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_oversize_images_and_pixel_bombs() {
        assert!(Chart::encode("chart", b"not png").is_err());
        let mut png = vec![0; 33];
        png[..16].copy_from_slice(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR");
        png[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
        png[20..24].copy_from_slice(&1u32.to_be_bytes());
        assert!(Chart::encode("chart", &png).is_err());
        png[16..20].copy_from_slice(&2u32.to_be_bytes());
        let content = Chart::encode("Example", &png).unwrap();
        assert_eq!(Chart::decode(&content).unwrap().1, png);
        let interactive =
            Chart::encode_openui("Example", &png, Some("root = Stack([])\n")).unwrap();
        assert_eq!(Chart::decode(&interactive).unwrap().1, png);
        assert!(Chart::encode_openui("Example", &png, Some(&"x".repeat(65537))).is_err());
        assert!(Chart::encode_openui("Example", &png, Some("bad\0source")).is_err());
        assert!(Chart::decode(&(content + " trailing")).is_none());
        assert!(Chart::encode("bad\ntitle", &png).is_err());
    }
}
