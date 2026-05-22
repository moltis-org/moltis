use {
    anyhow::{Result, anyhow},
    bytes::Bytes,
};

pub(crate) fn wav_from_s16le_mono(pcm: &[u8], sample_rate_hz: u32) -> Result<Bytes> {
    let data_len = u32::try_from(pcm.len())?;
    let byte_rate = sample_rate_hz
        .checked_mul(2)
        .ok_or_else(|| anyhow!("sample rate is too large for WAV output"))?;
    let riff_len = 36u32
        .checked_add(data_len)
        .ok_or_else(|| anyhow!("audio is too large for WAV output"))?;
    let mut wav = Vec::with_capacity(44 + pcm.len());

    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&riff_len.to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&sample_rate_hz.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(pcm);

    Ok(Bytes::from(wav))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wav_from_s16le_mono_writes_header_and_payload() -> Result<()> {
        let pcm = [0x01, 0x02, 0x03, 0x04];
        let wav = wav_from_s16le_mono(&pcm, 16_000)?;

        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into()?), 16_000);
        assert_eq!(u32::from_le_bytes(wav[28..32].try_into()?), 32_000);
        assert_eq!(u16::from_le_bytes(wav[32..34].try_into()?), 2);
        assert_eq!(u16::from_le_bytes(wav[34..36].try_into()?), 16);
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into()?), 4);
        assert_eq!(&wav[44..], &pcm);
        Ok(())
    }
}
