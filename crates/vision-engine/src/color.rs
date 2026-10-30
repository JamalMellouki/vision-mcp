//! Color parsing and conversion.

use serde::{Deserialize, Serialize};

static NAMED_COLORS: &[(&str, u32)] = &include!("named_colors.rs");

/// An sRGB color with straight alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Color {
    /// Red channel.
    pub(crate) r: u8,
    /// Green channel.
    pub(crate) g: u8,
    /// Blue channel.
    pub(crate) b: u8,
    /// Alpha channel.
    pub(crate) a: u8,
}

impl Color {
    /// Opaque black.
    pub(crate) const BLACK: Self = Self::rgb(0, 0, 0);
    /// Opaque white.
    pub(crate) const WHITE: Self = Self::rgb(255, 255, 255);
    /// Fully transparent.
    pub(crate) const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);

    /// Creates an opaque color.
    #[must_use]
    pub(crate) const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// Creates a color with an alpha channel.
    #[must_use]
    pub(crate) const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Returns the color with a new alpha value.
    #[must_use]
    pub(crate) const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }

    /// Converts the color to a premultiplied `tiny-skia` color.
    #[must_use]
    pub(crate) fn to_tiny_skia(self) -> tiny_skia::Color {
        tiny_skia::Color::from_rgba8(self.r, self.g, self.b, self.a)
    }

    /// Parses a CSS color value.
    ///
    /// Supported forms: named colors, `#rgb`, `#rgba`, `#rrggbb`,
    /// `#rrggbbaa`, `rgb()`, and `rgba()`.
    #[must_use]
    pub(crate) fn parse(input: &str) -> Option<Self> {
        let input = input.trim();
        if input.is_empty() {
            return None;
        }
        let lower = input.to_ascii_lowercase();
        if lower == "transparent" {
            return Some(Self::TRANSPARENT);
        }
        if let Some(hex) = lower.strip_prefix('#') {
            return Self::parse_hex(hex);
        }
        if lower.starts_with("rgb(") && lower.ends_with(')') {
            return Self::parse_rgb(&lower[4..lower.len() - 1], false);
        }
        if lower.starts_with("rgba(") && lower.ends_with(')') {
            return Self::parse_rgb(&lower[5..lower.len() - 1], true);
        }
        NAMED_COLORS
            .binary_search_by_key(&lower.as_str(), |(name, _)| name)
            .ok()
            .map(|index| {
                let value = NAMED_COLORS[index].1;
                Self::rgb(
                    ((value >> 16) & 0xff) as u8,
                    ((value >> 8) & 0xff) as u8,
                    (value & 0xff) as u8,
                )
            })
    }

    fn parse_hex(hex: &str) -> Option<Self> {
        let digits: Vec<u8> = hex
            .bytes()
            .map(|byte| match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'a'..=b'f' => Some(byte - b'a' + 10),
                b'A'..=b'F' => Some(byte - b'A' + 10),
                _ => None,
            })
            .collect::<Option<_>>()?;
        match digits.len() {
            3 => Some(Self::rgb(digits[0] * 17, digits[1] * 17, digits[2] * 17)),
            4 => Some(Self::rgba(
                digits[0] * 17,
                digits[1] * 17,
                digits[2] * 17,
                digits[3] * 17,
            )),
            6 => Some(Self::rgb(
                digits[0] * 16 + digits[1],
                digits[2] * 16 + digits[3],
                digits[4] * 16 + digits[5],
            )),
            8 => Some(Self::rgba(
                digits[0] * 16 + digits[1],
                digits[2] * 16 + digits[3],
                digits[4] * 16 + digits[5],
                digits[6] * 16 + digits[7],
            )),
            _ => None,
        }
    }

    fn parse_rgb(input: &str, has_alpha: bool) -> Option<Self> {
        let parts: Vec<&str> = input.split(',').map(str::trim).collect();
        let expected = if has_alpha { 4 } else { 3 };
        if parts.len() != expected {
            return None;
        }
        let channel = |value: &str| -> Option<u8> {
            if let Some(percent) = value.strip_suffix('%') {
                let number = percent.parse::<f32>().ok()?;
                Some((number.clamp(0.0, 100.0) * 2.55).round() as u8)
            } else {
                value
                    .parse::<f32>()
                    .ok()
                    .map(|number| number.clamp(0.0, 255.0) as u8)
            }
        };
        let alpha = if has_alpha {
            let value = parts[3];
            if let Some(percent) = value.strip_suffix('%') {
                let number = percent.parse::<f32>().ok()?;
                (number.clamp(0.0, 100.0) * 2.55).round() as u8
            } else {
                let number = value.parse::<f32>().ok()?;
                (number.clamp(0.0, 1.0) * 255.0).round() as u8
            }
        } else {
            255
        };
        Some(Self::rgba(
            channel(parts[0])?,
            channel(parts[1])?,
            channel(parts[2])?,
            alpha,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_forms() {
        assert_eq!(Color::parse("#0f0"), Some(Color::rgb(0, 255, 0)));
        assert_eq!(Color::parse("#ff000080"), Some(Color::rgba(255, 0, 0, 128)));
        assert_eq!(Color::parse("#abc"), Some(Color::rgb(170, 187, 204)));
    }

    #[test]
    fn parses_functional_forms() {
        assert_eq!(Color::parse("rgb(255, 0, 0)"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(
            Color::parse("rgba(0, 0, 0, 0.5)"),
            Some(Color::rgba(0, 0, 0, 128))
        );
        assert_eq!(Color::parse("transparent"), Some(Color::TRANSPARENT));
    }

    #[test]
    fn parses_named_colors() {
        assert_eq!(
            Color::parse("rebeccapurple"),
            Some(Color::rgb(102, 51, 153))
        );
        assert_eq!(Color::parse("white"), Some(Color::WHITE));
        assert_eq!(Color::parse("not-a-color"), None);
    }
}
