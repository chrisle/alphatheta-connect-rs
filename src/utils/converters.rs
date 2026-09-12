//! Bit-twiddling helpers for waveform data.

use crate::types::{WaveformHD, WaveformHDSegment};

/// Extracts a specific bitmask, shifting it down to the mask's lowest bit.
pub fn extract_bit_mask(val: u32, mask: u32) -> u32 {
    (val & mask) >> mask.trailing_zeros()
}

/// Pioneer colors are 3 bits, convert this to a percentage.
pub fn extract_color(val: u32, mask: u32) -> f64 {
    extract_bit_mask(val, mask) as f64 / 0b111 as f64
}

/// Convert raw waveform HD data into the structured [`WaveformHD`] type.
pub fn convert_waveform_hd_data(data: &[u8]) -> WaveformHD {
    // Two byte bit representation for the color waveform.
    //
    // | f  e  d | c  b  a | 9  8  7 | 6  5  4  3  2 | 1   0 |
    // [   red   |  green  |   blue  |     height    | ~ | ~ ]
    const RED_MASK: u32 = 0b11100000_00000000;
    const GREEN_MASK: u32 = 0b00011100_00000000;
    const BLUE_MASK: u32 = 0b00000011_10000000;
    const HEIGHT_MASK: u32 = 0b00000000_01111100;

    data.chunks_exact(2)
        .map(|c| u32::from(u16::from_be_bytes([c[0], c[1]])))
        .map(|v| WaveformHDSegment {
            height: extract_bit_mask(v, HEIGHT_MASK) as u8,
            color: [extract_color(v, RED_MASK), extract_color(v, GREEN_MASK), extract_color(v, BLUE_MASK)],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_masks() {
        assert_eq!(extract_bit_mask(0b1110_0000, 0b1110_0000), 0b111);
        assert_eq!(extract_bit_mask(0b0001_1111, 0b0001_1111), 31);
        assert_eq!(extract_color(0b1110_0000, 0b1110_0000), 1.0);
    }

    #[test]
    fn converts_hd_segments() {
        // red=7, green=0, blue=7, height=31
        let word: u16 = 0b1110_0011_1111_1100;
        let hd = convert_waveform_hd_data(&word.to_be_bytes());
        assert_eq!(hd.len(), 1);
        assert_eq!(hd[0].height, 31);
        assert_eq!(hd[0].color, [1.0, 0.0, 1.0]);
    }
}
