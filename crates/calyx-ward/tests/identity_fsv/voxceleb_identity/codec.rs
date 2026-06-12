use super::CALYX_WARD_VOXCELEB_BAD_WAV;
use super::data::FsvError;

#[derive(Debug)]
pub(super) struct DecodedWav {
    pub(super) sample_rate: u32,
    pub(super) frames: usize,
    pub(super) samples: Vec<f32>,
}

pub(super) fn decode_wav_pcm16_mono(bytes: &[u8]) -> Result<DecodedWav, FsvError> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(FsvError::new(CALYX_WARD_VOXCELEB_BAD_WAV, "not RIFF/WAVE"));
    }
    let mut fmt = None;
    let mut data = None;
    let mut offset = 12;
    while offset + 8 <= bytes.len() {
        let id = &bytes[offset..offset + 4];
        let size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
        let start = offset + 8;
        let end = start.saturating_add(size);
        if end > bytes.len() {
            break;
        }
        match id {
            b"fmt " => fmt = Some(&bytes[start..end]),
            b"data" => data = Some(&bytes[start..end]),
            _ => {}
        }
        offset = end + (size % 2);
    }
    let fmt = fmt.ok_or_else(|| FsvError::new(CALYX_WARD_VOXCELEB_BAD_WAV, "missing fmt chunk"))?;
    let data =
        data.ok_or_else(|| FsvError::new(CALYX_WARD_VOXCELEB_BAD_WAV, "missing data chunk"))?;
    if fmt.len() < 16 {
        return Err(FsvError::new(
            CALYX_WARD_VOXCELEB_BAD_WAV,
            "short fmt chunk",
        ));
    }
    let format = u16::from_le_bytes(fmt[0..2].try_into().unwrap());
    let channels = u16::from_le_bytes(fmt[2..4].try_into().unwrap());
    let sample_rate = u32::from_le_bytes(fmt[4..8].try_into().unwrap());
    let bits = u16::from_le_bytes(fmt[14..16].try_into().unwrap());
    if format != 1 || channels != 1 || bits != 16 || data.is_empty() || data.len() % 2 != 0 {
        return Err(FsvError::new(
            CALYX_WARD_VOXCELEB_BAD_WAV,
            format!(
                "expected PCM16 mono data, got format={format} channels={channels} bits={bits}"
            ),
        ));
    }
    let samples = data
        .chunks_exact(2)
        .map(|chunk| i16::from_le_bytes(chunk.try_into().unwrap()) as f32 / 32768.0)
        .collect::<Vec<_>>();
    Ok(DecodedWav {
        sample_rate,
        frames: samples.len(),
        samples,
    })
}
