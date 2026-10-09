//! Colours as Pine thinks of them: RGB plus transparency 0–100.

use serde::{Deserialize, Serialize};

/// 0xRRGGBBAA.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rgba(pub u32);

impl Rgba {
    /// Opaque from 0xRRGGBB.
    pub const fn hex(rgb: u32) -> Rgba {
        Rgba((rgb << 8) | 0xFF)
    }

    /// Pine's `color.new(c, transp)` / `color.rgb(r, g, b, transp)`:
    /// transparency 0 (opaque) to 100 (invisible).
    pub const fn transp(self, transp: u32) -> Rgba {
        let t = if transp > 100 { 100 } else { transp };
        let a = ((100 - t) * 255 + 50) / 100;
        Rgba((self.0 & 0xFFFF_FF00) | a)
    }

    pub const fn r(self) -> u8 {
        (self.0 >> 24) as u8
    }
    pub const fn g(self) -> u8 {
        (self.0 >> 16) as u8
    }
    pub const fn b(self) -> u8 {
        (self.0 >> 8) as u8
    }
    pub const fn a(self) -> u8 {
        self.0 as u8
    }

    /// Transparency 0–100, as shown in the colour picker.
    pub fn transparency(self) -> u32 {
        100 - (self.a() as u32 * 100 + 127) / 255
    }
}

/// TradingView's palette as used by its built-in indicators.
pub mod tv {
    use super::Rgba;
    pub const BLUE: Rgba = Rgba::hex(0x2962FF);
    pub const RED: Rgba = Rgba::hex(0xF23645);
    pub const GREEN: Rgba = Rgba::hex(0x089981);
    pub const ORANGE: Rgba = Rgba::hex(0xFF6D00);
    pub const PURPLE: Rgba = Rgba::hex(0x7E57C2);
    pub const GRAY: Rgba = Rgba::hex(0x787B86);
    pub const YELLOW: Rgba = Rgba::hex(0xE2CC00);
    pub const PINK: Rgba = Rgba::hex(0xE91E63);
    pub const DARK_RED: Rgba = Rgba::hex(0xB71C1C);
    pub const LEAF: Rgba = Rgba::hex(0x43A047);
    pub const TEAL: Rgba = Rgba::hex(0x22AB94);
    pub const AMBER: Rgba = Rgba::hex(0xFB8C00);
    pub const MAGENTA: Rgba = Rgba::hex(0xF50057);
    /// The light-blue band fill (`color.rgb(33, 150, 243, 95)`).
    pub const BAND: Rgba = Rgba::hex(0x2196F3).transp(95);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparency_round_trips() {
        let c = Rgba::hex(0x7E57C2).transp(90);
        assert_eq!(c.a(), 26);
        assert_eq!(c.transparency(), 90);
        assert_eq!(Rgba::hex(0x123456).transparency(), 0);
        assert_eq!((c.r(), c.g(), c.b()), (0x7E, 0x57, 0xC2));
    }
}
