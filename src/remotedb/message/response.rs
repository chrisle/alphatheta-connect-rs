//! Converters from response message arguments to structured data.

use crate::entities::{CueAndLoop, CueColor, HotcueButton};
use crate::localdb::utils::make_cue_loop_entry;
use crate::remotedb::fields::Field;
use crate::remotedb::message::item::{fields_to_item, Item};
use crate::remotedb::message::types::{response, MessageType};
use crate::types::{Beat, BeatGrid, WaveformDetailed, WaveformHD, WaveformPreview, WaveformSegment};
use crate::utils::converters::{convert_waveform_hd_data, extract_bit_mask, extract_color};
use crate::{Error, Result};

/// The structured representation of a response message.
#[derive(Debug, Clone, PartialEq)]
pub enum ResponseData {
    /// Setup success, which primarily includes the number of items available
    /// upon the next request.
    Success {
        items_available: u32,
    },
    /// Responses with no data (error, menu header, menu footer).
    Null,
    MenuItem(Item),
    /// Artwork bytes. Empty for empty artwork.
    Artwork(Vec<u8>),
    BeatGrid(BeatGrid),
    CueAndLoop(Vec<CueAndLoop>),
    WaveformPreview(WaveformPreview),
    WaveformDetailed(WaveformDetailed),
    WaveformHD(WaveformHD),
    AdvCueAndLoops(Vec<CueAndLoop>),
}

fn binary_arg(args: &[Field], i: usize) -> Result<&[u8]> {
    args.get(i).and_then(Field::as_binary).ok_or_else(|| Error::RemoteDb(format!("expected binary field at argument {i}")))
}

fn u16_le(d: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([d[at], d[at + 1]])
}

fn u32_le(d: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]])
}

/// Converts setup success messages.
fn convert_success(args: &[Field]) -> Result<ResponseData> {
    let items_available = args.get(1).and_then(Field::as_number).ok_or_else(|| Error::RemoteDb("missing item count".into()))?;
    Ok(ResponseData::Success { items_available })
}

/// Converts artwork to bytes. Will be empty for empty artwork.
fn convert_artwork(args: &[Field]) -> Result<ResponseData> {
    Ok(ResponseData::Artwork(binary_arg(args, 3)?.to_vec()))
}

/// Converts the beat grid binary response to a BeatGrid.
fn convert_beat_grid(args: &[Field]) -> Result<ResponseData> {
    const BEATGRID_START: usize = 0x14;
    let data = binary_arg(args, 3)?;
    let data = data.get(BEATGRID_START..).unwrap_or(&[]);

    let grid = data
        .chunks_exact(0x10)
        .map(|entry| Beat { offset: u32_le(entry, 4), bpm: f64::from(u16_le(entry, 2)) / 100.0, count: entry[0] })
        .collect();

    Ok(ResponseData::BeatGrid(grid))
}

/// Converts preview waveform data.
fn convert_waveform_preview(args: &[Field]) -> Result<ResponseData> {
    let data = binary_arg(args, 3)?;

    // TODO: The last 100 bytes in the data array is a tiny waveform preview
    const PREVIEW_DATA_LEN: usize = 800;

    let preview = (0..PREVIEW_DATA_LEN)
        .step_by(2)
        .map(|at| WaveformSegment {
            height: data.get(at).copied().unwrap_or(0),
            whiteness: f64::from(data.get(at + 1).copied().unwrap_or(0)) / 7.0,
        })
        .collect();

    Ok(ResponseData::WaveformPreview(preview))
}

/// Converts detailed waveform data.
fn convert_waveform_detailed(args: &[Field]) -> Result<ResponseData> {
    let data = binary_arg(args, 3)?;

    // Every byte represents one segment of the waveform, and there are 150
    // segments per second of audio. (These seem to correspond to 'half
    // frames' following the seconds in the player display.) Each byte
    // encodes both a color and height.
    //
    // |  7  6  5  |  4  3  2  1  0 |
    // [ whiteness |     height     ]
    const WHITENESS_MASK: u32 = 0b11100000;
    const HEIGHT_MASK: u32 = 0b00011111;

    let detailed = data
        .iter()
        .map(|&b| WaveformSegment {
            height: extract_bit_mask(u32::from(b), HEIGHT_MASK) as u8,
            whiteness: extract_color(u32::from(b), WHITENESS_MASK),
        })
        .collect();

    Ok(ResponseData::WaveformDetailed(detailed))
}

/// Converts HD waveform data.
fn convert_waveform_hd(args: &[Field]) -> Result<ResponseData> {
    // TODO: Verify this 0x34 offset is correct
    const WAVEFORM_START: usize = 0x34;
    let data = binary_arg(args, 3)?;
    let data = data.get(WAVEFORM_START..).unwrap_or(&[]);

    // TODO: This response is also used for the HD waveform previews, however
    // those have a much more complex data structure.
    Ok(ResponseData::WaveformHD(convert_waveform_hd_data(data)))
}

/// Converts old-style cue / loop / hotcue / hotloop data.
fn convert_cue_and_loops(args: &[Field]) -> Result<ResponseData> {
    let data = binary_arg(args, 3)?;

    let cues = data
        .chunks_exact(0x24)
        .filter_map(|entry| {
            let is_loop = entry[0] != 0;
            let is_cue = entry[1] != 0;
            let button = if entry[2] == 0 { None } else { HotcueButton::from_u8(entry[2]) };

            let offset_in_frames = u32_le(entry, 0x0c);
            let length_in_frames = u32_le(entry, 0x10).wrapping_sub(offset_in_frames);

            // NOTE: The offset and length are reported as 1/150th second
            //       increments. We convert these to milliseconds here.
            let offset = f64::from(offset_in_frames) / 150.0 * 1000.0;
            let length = f64::from(length_in_frames) / 150.0 * 1000.0;

            make_cue_loop_entry(is_cue, is_loop, offset, length, button)
        })
        .collect();

    Ok(ResponseData::CueAndLoop(cues))
}

/// Converts new-style cue / loop / hotcue / hotloop data, including labels
/// and colors.
fn convert_adv_cue_and_loops(args: &[Field]) -> Result<ResponseData> {
    let data = binary_arg(args, 3)?;

    let mut entries: Vec<&[u8]> = Vec::new();
    let mut offset = 0usize;
    while offset + 4 <= data.len() {
        let length = u32_le(data, offset) as usize;
        if length == 0 {
            break;
        }
        let end = (offset + length).min(data.len());
        entries.push(&data[offset..end]);
        offset += length;
    }

    let cues = entries
        .into_iter()
        .filter_map(|entry| {
            if entry.len() < 0x14 {
                return None;
            }
            // Deleted cue point
            if entry[6] == 0x00 {
                return None;
            }

            // The layout here is minorly different from the basic cue and
            // loops, so we unfortunately cannot reuse that logic.
            let button = if entry[4] == 0 { None } else { HotcueButton::from_u8(entry[4]) };
            let is_cue = entry[6] == 0x01;
            let is_loop = entry[6] == 0x02;

            let offset_in_frames = u32_le(entry, 0x0c);
            let length_in_frames = u32_le(entry, 0x10).wrapping_sub(offset_in_frames);

            // NOTE: The offset and length are reported as 1/150th second
            //       increments. We convert these to milliseconds here.
            let offset = f64::from(offset_in_frames) / 150.0 * 1000.0;
            let length = f64::from(length_in_frames) / 150.0 * 1000.0;

            let basic = make_cue_loop_entry(is_cue, is_loop, offset, length, button)?;

            // It seems the label may not always be included, if the entry is
            // only 0x38 bytes long, exclude color and comment
            if entry.len() == 0x38 || entry.len() < 0x4a {
                return Some(basic);
            }

            let label_byte_length = usize::from(u16_le(entry, 0x48));
            let label_end = (0x4a + label_byte_length).min(entry.len());
            let label_bytes = &entry[0x4a..label_end];
            let label_bytes = &label_bytes[..label_bytes.len().saturating_sub(2)];
            let label_units: Vec<u16> = label_bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            let label = String::from_utf16_lossy(&label_units);

            let color = entry.get(0x4a + label_byte_length + 0x04).map(|c| CueColor::from_u8(*c));

            Some(basic.with_label_color(Some(label), color))
        })
        .collect();

    Ok(ResponseData::AdvCueAndLoops(cues))
}

/// Convert the arguments of a response of type `message_type`.
pub fn response_transform(message_type: MessageType, args: &[Field]) -> Result<ResponseData> {
    match message_type {
        response::SUCCESS => convert_success(args),
        response::ERROR | response::MENU_HEADER | response::MENU_FOOTER => Ok(ResponseData::Null),
        response::MENU_ITEM => Ok(ResponseData::MenuItem(fields_to_item(args))),
        response::ARTWORK => convert_artwork(args),
        response::BEAT_GRID => convert_beat_grid(args),
        response::CUE_AND_LOOP => convert_cue_and_loops(args),
        response::WAVEFORM_PREVIEW => convert_waveform_preview(args),
        response::WAVEFORM_DETAILED => convert_waveform_detailed(args),
        response::WAVEFORM_HD => convert_waveform_hd(args),
        response::ADV_CUE_AND_LOOPS => convert_adv_cue_and_loops(args),
        other => Err(Error::RemoteDb(format!("Representation of non-responses is not currently supported (0x{other:04x})"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beat_grid_conversion() {
        let mut data = vec![0u8; 0x14];
        // one entry: count 1, bpm 128.00, offset 500ms
        let mut entry = [0u8; 0x10];
        entry[0] = 1;
        entry[2..4].copy_from_slice(&12800u16.to_le_bytes());
        entry[4..8].copy_from_slice(&500u32.to_le_bytes());
        data.extend_from_slice(&entry);
        let args = vec![Field::UInt32(0), Field::UInt32(0), Field::UInt32(0), Field::Binary(data)];
        let ResponseData::BeatGrid(grid) = response_transform(response::BEAT_GRID, &args).unwrap() else { panic!() };
        assert_eq!(grid, vec![Beat { offset: 500, count: 1, bpm: 128.0 }]);
    }

    #[test]
    fn cue_and_loops_conversion() {
        let mut entry = [0u8; 0x24];
        entry[1] = 1; // cue
        entry[0x0c..0x10].copy_from_slice(&150u32.to_le_bytes());
        entry[0x10..0x14].copy_from_slice(&150u32.to_le_bytes());
        let mut hot = [0u8; 0x24];
        hot[0] = 1; // loop
        hot[2] = 2; // button B
        hot[0x0c..0x10].copy_from_slice(&300u32.to_le_bytes());
        hot[0x10..0x14].copy_from_slice(&450u32.to_le_bytes());
        let mut data = entry.to_vec();
        data.extend_from_slice(&hot);
        let args = vec![Field::UInt32(0), Field::UInt32(0), Field::UInt32(0), Field::Binary(data)];
        let ResponseData::CueAndLoop(cues) = response_transform(response::CUE_AND_LOOP, &args).unwrap() else { panic!() };
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0], CueAndLoop::CuePoint { offset: 1000.0, label: None, color: None });
        assert_eq!(
            cues[1],
            CueAndLoop::HotLoop { offset: 2000.0, length: 1000.0, button: HotcueButton::B, label: None, color: None }
        );
    }

    #[test]
    fn waveform_detailed_conversion() {
        let args = vec![Field::UInt32(0), Field::UInt32(0), Field::UInt32(0), Field::Binary(vec![0b1111_1111, 0b0000_0001])];
        let ResponseData::WaveformDetailed(w) = response_transform(response::WAVEFORM_DETAILED, &args).unwrap() else { panic!() };
        assert_eq!(w[0], WaveformSegment { height: 31, whiteness: 1.0 });
        assert_eq!(w[1], WaveformSegment { height: 1, whiteness: 0.0 });
    }

    #[test]
    fn success_conversion() {
        let args = vec![Field::UInt32(0), Field::UInt32(12)];
        assert_eq!(response_transform(response::SUCCESS, &args).unwrap(), ResponseData::Success { items_available: 12 });
    }
}
