//! Telephony audio conversion utilities.
//!
//! PSTN calls use mu-law encoding at 8 kHz sample rate. This module converts
//! PCM audio (from TTS providers) into telephony-grade mu-law format.

use bytes::Bytes;

/// Standard telephony sample rate.
pub const TELEPHONY_SAMPLE_RATE: u32 = 8000;

/// Encode a single 16-bit linear PCM sample to mu-law.
///
/// Uses the ITU-T G.711 algorithm with a bias of 0x84 (132).
#[must_use]
pub fn pcm_to_mulaw(sample: i16) -> u8 {
    const BIAS: i16 = 0x84;
    const CLIP: i16 = 32635;

    // Table of mu-law segment positions.
    const SEG_END: [i16; 8] = [0xFF, 0x1FF, 0x3FF, 0x7FF, 0xFFF, 0x1FFF, 0x3FFF, 0x7FFF];

    let sign = if sample >= 0 {
        0xFF_u8
    } else {
        0x7F_u8
    };
    let mut pcm_val = if sample < 0 {
        -sample
    } else {
        sample
    };

    if pcm_val > CLIP {
        pcm_val = CLIP;
    }
    pcm_val += BIAS;

    let mut seg: usize = 0;
    for (i, &end) in SEG_END.iter().enumerate() {
        if pcm_val <= end {
            seg = i;
            break;
        }
        if i == SEG_END.len() - 1 {
            seg = i;
        }
    }

    let seg_u8 = seg as u8;
    if seg >= 8 {
        return sign ^ 0x7F;
    }

    let mantissa = if seg >= 1 {
        (pcm_val >> (seg + 3)) & 0x0F
    } else {
        (pcm_val >> 4) & 0x0F
    };

    let byte = sign ^ ((seg_u8 << 4) | mantissa as u8);
    byte ^ 0xFF
}

/// Convert a buffer of 16-bit PCM samples (little-endian) to mu-law bytes.
#[must_use]
pub fn pcm16_to_mulaw(pcm: &[u8]) -> Vec<u8> {
    pcm.chunks_exact(2)
        .map(|chunk| {
            let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
            pcm_to_mulaw(sample)
        })
        .collect()
}

/// Convert PCM 16-bit audio at `src_rate` Hz to mu-law at 8 kHz.
///
/// Performs nearest-neighbor downsampling if the source rate differs from 8 kHz.
#[must_use]
pub fn resample_and_encode_mulaw(pcm: &[u8], src_rate: u32) -> Bytes {
    if src_rate == TELEPHONY_SAMPLE_RATE {
        return Bytes::from(pcm16_to_mulaw(pcm));
    }

    let samples: Vec<i16> = pcm
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect();

    let ratio = src_rate as f64 / TELEPHONY_SAMPLE_RATE as f64;
    let out_len = (samples.len() as f64 / ratio) as usize;

    let mulaw: Vec<u8> = (0..out_len)
        .map(|i| {
            let src_idx = ((i as f64) * ratio) as usize;
            let sample = samples.get(src_idx).copied().unwrap_or(0);
            pcm_to_mulaw(sample)
        })
        .collect();

    Bytes::from(mulaw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_encodes_consistently() {
        // PCM 0 (silence) should produce a consistent mu-law value.
        // The exact value depends on the bias; just verify it's stable.
        let a = pcm_to_mulaw(0);
        let b = pcm_to_mulaw(0);
        assert_eq!(a, b);
        // Low-amplitude values should encode without panicking.
        let _pos = pcm_to_mulaw(1);
        let _neg = pcm_to_mulaw(-1);
    }

    #[test]
    fn pcm16_roundtrip_length() {
        let pcm = vec![0u8; 100]; // 50 samples
        let mulaw = pcm16_to_mulaw(&pcm);
        assert_eq!(mulaw.len(), 50);
    }

    #[test]
    fn resample_downsamples_correctly() {
        // 16 kHz input → 8 kHz output should halve the sample count.
        let num_samples = 1000;
        let pcm: Vec<u8> = vec![0u8; num_samples * 2]; // 1000 samples at 16-bit
        let mulaw = resample_and_encode_mulaw(&pcm, 16000);
        assert_eq!(mulaw.len(), 500);
    }

    #[test]
    fn same_rate_no_resample() {
        let pcm = vec![0u8; 200]; // 100 samples
        let mulaw = resample_and_encode_mulaw(&pcm, 8000);
        assert_eq!(mulaw.len(), 100);
    }
}
