//! Image processing operations for LLM-ready media.
//!
//! Provides utilities for resizing and optimizing images before sending to LLMs.
//! Handles dimension limits, file size constraints, and format conversion.

use std::io::Cursor;

use {
    anyhow::{Context, Result},
    image::{DynamicImage, GenericImageView, ImageFormat, ImageReader},
};

/// Default maximum dimension (width or height) for images.
/// Claude API allows 8000px for single images, 2000px for multi-image requests.
/// We use 1568px as a conservative default that works well with both providers.
pub const DEFAULT_MAX_DIMENSION: u32 = 1568;

/// Maximum file size for Anthropic API (5MB).
pub const MAX_FILE_SIZE_BYTES: usize = 5 * 1024 * 1024;

/// JPEG quality for resized images (0-100).
pub const DEFAULT_JPEG_QUALITY: u8 = 85;

/// Image metadata.
#[derive(Debug, Clone)]
pub struct ImageMetadata {
    pub width: u32,
    pub height: u32,
    pub format: Option<ImageFormat>,
}

/// Result of image optimization.
#[derive(Debug)]
pub struct OptimizedImage {
    /// The optimized image data.
    pub data: Vec<u8>,
    /// MIME type of the output image.
    pub media_type: String,
    /// Original dimensions.
    pub original_width: u32,
    pub original_height: u32,
    /// Final dimensions after resizing.
    pub final_width: u32,
    pub final_height: u32,
    /// Whether the image was resized.
    pub was_resized: bool,
}

/// Get metadata about an image without fully decoding it.
pub fn get_image_metadata(data: &[u8]) -> Result<ImageMetadata> {
    let reader = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .context("failed to guess image format")?;

    let format = reader.format();
    let (width, height) = reader
        .into_dimensions()
        .context("failed to read image dimensions")?;

    Ok(ImageMetadata {
        width,
        height,
        format,
    })
}

/// Check if an image needs resizing based on dimensions or file size.
pub fn needs_optimization(data: &[u8], max_dimension: u32) -> Result<bool> {
    // Check file size
    if data.len() > MAX_FILE_SIZE_BYTES {
        return Ok(true);
    }

    // Check dimensions
    let meta = get_image_metadata(data)?;
    if meta.width > max_dimension || meta.height > max_dimension {
        return Ok(true);
    }

    Ok(false)
}

/// Resize image to fit within max dimensions while preserving aspect ratio.
/// Returns the original data if no resizing is needed.
pub fn resize_image(data: &[u8], max_width: u32, max_height: u32) -> Result<Vec<u8>> {
    let img = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .context("failed to guess image format")?
        .decode()
        .context("failed to decode image")?;

    let (orig_width, orig_height) = img.dimensions();

    // Check if resizing is needed
    if orig_width <= max_width && orig_height <= max_height {
        return Ok(data.to_vec());
    }

    // Calculate new dimensions preserving aspect ratio
    let ratio = (max_width as f64 / orig_width as f64).min(max_height as f64 / orig_height as f64);
    let new_width = (orig_width as f64 * ratio).round() as u32;
    let new_height = (orig_height as f64 * ratio).round() as u32;

    // Resize using Lanczos3 filter for high quality
    let resized = img.resize(new_width, new_height, image::imageops::FilterType::Lanczos3);

    // Encode as JPEG
    let mut output = Cursor::new(Vec::new());
    resized
        .write_to(&mut output, ImageFormat::Jpeg)
        .context("failed to encode resized image")?;

    Ok(output.into_inner())
}

/// Optimize an image for LLM consumption.
///
/// This function:
/// 1. Checks if the image exceeds dimension or file size limits
/// 2. Resizes if needed, preserving aspect ratio
/// 3. Converts to JPEG for efficiency (unless PNG with transparency is needed)
/// 4. Ensures the output is under the API size limit
///
/// # Arguments
/// * `data` - Raw image bytes
/// * `max_dimension` - Maximum width or height (default: 1568)
///
/// # Returns
/// * `OptimizedImage` with the processed data and metadata
pub fn optimize_for_llm(data: &[u8], max_dimension: Option<u32>) -> Result<OptimizedImage> {
    let max_dim = max_dimension.unwrap_or(DEFAULT_MAX_DIMENSION);

    let img = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .context("failed to guess image format")?
        .decode()
        .context("failed to decode image")?;

    let (orig_width, orig_height) = img.dimensions();
    let needs_resize =
        orig_width > max_dim || orig_height > max_dim || data.len() > MAX_FILE_SIZE_BYTES;

    if !needs_resize {
        // Determine media type from format
        let format = ImageReader::new(Cursor::new(data))
            .with_guessed_format()
            .ok()
            .and_then(|r| r.format());
        let media_type = format_to_media_type(format).to_string();

        return Ok(OptimizedImage {
            data: data.to_vec(),
            media_type,
            original_width: orig_width,
            original_height: orig_height,
            final_width: orig_width,
            final_height: orig_height,
            was_resized: false,
        });
    }

    // Resize the image
    let (final_width, final_height, resized) = resize_to_fit(&img, max_dim);

    // Check if image has alpha channel (transparency)
    let has_alpha = matches!(
        img,
        DynamicImage::ImageRgba8(_)
            | DynamicImage::ImageRgba16(_)
            | DynamicImage::ImageRgba32F(_)
            | DynamicImage::ImageLumaA8(_)
            | DynamicImage::ImageLumaA16(_)
    );

    // Try encoding with target quality
    let (output_data, media_type) = if has_alpha {
        // Keep as PNG for transparency
        let mut output = Cursor::new(Vec::new());
        resized
            .write_to(&mut output, ImageFormat::Png)
            .context("failed to encode as PNG")?;
        (output.into_inner(), "image/png")
    } else {
        // Convert to JPEG for efficiency
        encode_jpeg_with_quality(&resized, DEFAULT_JPEG_QUALITY)?
    };

    // If still too large, try progressive quality reduction
    let final_data = if output_data.len() > MAX_FILE_SIZE_BYTES && !has_alpha {
        reduce_size_to_fit(&resized, MAX_FILE_SIZE_BYTES)?
    } else {
        output_data
    };

    Ok(OptimizedImage {
        data: final_data,
        media_type: media_type.to_string(),
        original_width: orig_width,
        original_height: orig_height,
        final_width,
        final_height,
        was_resized: true,
    })
}

/// Resize image to fit within max dimension, preserving aspect ratio.
fn resize_to_fit(img: &DynamicImage, max_dimension: u32) -> (u32, u32, DynamicImage) {
    let (width, height) = img.dimensions();

    if width <= max_dimension && height <= max_dimension {
        return (width, height, img.clone());
    }

    let ratio = if width > height {
        max_dimension as f64 / width as f64
    } else {
        max_dimension as f64 / height as f64
    };

    let new_width = (width as f64 * ratio).round() as u32;
    let new_height = (height as f64 * ratio).round() as u32;

    let resized = img.resize(new_width, new_height, image::imageops::FilterType::Lanczos3);
    (new_width, new_height, resized)
}

/// Encode image as JPEG with specified quality.
fn encode_jpeg_with_quality(img: &DynamicImage, quality: u8) -> Result<(Vec<u8>, &'static str)> {
    let mut output = Cursor::new(Vec::new());
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, quality);
    img.write_with_encoder(encoder)
        .context("failed to encode as JPEG")?;
    Ok((output.into_inner(), "image/jpeg"))
}

/// Progressively reduce quality until image fits within size limit.
fn reduce_size_to_fit(img: &DynamicImage, max_bytes: usize) -> Result<Vec<u8>> {
    // Try progressively lower quality
    for quality in [80, 70, 60, 50, 40, 30] {
        let (data, _) = encode_jpeg_with_quality(img, quality)?;
        if data.len() <= max_bytes {
            return Ok(data);
        }
    }

    // If still too large, resize further
    let (width, height) = img.dimensions();
    let smaller_dim = (width.min(height) as f64 * 0.75).round() as u32;

    if smaller_dim < 256 {
        anyhow::bail!("image cannot be reduced to fit within size limit");
    }

    let resized = img.resize(
        smaller_dim,
        smaller_dim,
        image::imageops::FilterType::Lanczos3,
    );
    reduce_size_to_fit(&resized, max_bytes)
}

/// Convert ImageFormat to MIME type string.
fn format_to_media_type(format: Option<ImageFormat>) -> &'static str {
    match format {
        Some(ImageFormat::Jpeg) => "image/jpeg",
        Some(ImageFormat::Png) => "image/png",
        Some(ImageFormat::WebP) => "image/webp",
        Some(ImageFormat::Gif) => "image/gif",
        Some(ImageFormat::Bmp) => "image/bmp",
        _ => "image/jpeg", // Default to JPEG for unknown formats
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    // 1x1 red pixel JPEG
    const TINY_JPEG: &[u8] = &[
        0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00,
        0x01, 0x00, 0x01, 0x00, 0x00, 0xFF, 0xDB, 0x00, 0x43, 0x00, 0x08, 0x06, 0x06, 0x07, 0x06,
        0x05, 0x08, 0x07, 0x07, 0x07, 0x09, 0x09, 0x08, 0x0A, 0x0C, 0x14, 0x0D, 0x0C, 0x0B, 0x0B,
        0x0C, 0x19, 0x12, 0x13, 0x0F, 0x14, 0x1D, 0x1A, 0x1F, 0x1E, 0x1D, 0x1A, 0x1C, 0x1C, 0x20,
        0x24, 0x2E, 0x27, 0x20, 0x22, 0x2C, 0x23, 0x1C, 0x1C, 0x28, 0x37, 0x29, 0x2C, 0x30, 0x31,
        0x34, 0x34, 0x34, 0x1F, 0x27, 0x39, 0x3D, 0x38, 0x32, 0x3C, 0x2E, 0x33, 0x34, 0x32, 0xFF,
        0xC0, 0x00, 0x0B, 0x08, 0x00, 0x01, 0x00, 0x01, 0x01, 0x01, 0x11, 0x00, 0xFF, 0xC4, 0x00,
        0x1F, 0x00, 0x00, 0x01, 0x05, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B,
        0xFF, 0xC4, 0x00, 0xB5, 0x10, 0x00, 0x02, 0x01, 0x03, 0x03, 0x02, 0x04, 0x03, 0x05, 0x05,
        0x04, 0x04, 0x00, 0x00, 0x01, 0x7D, 0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21,
        0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xA1, 0x08,
        0x23, 0x42, 0xB1, 0xC1, 0x15, 0x52, 0xD1, 0xF0, 0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0A,
        0x16, 0x17, 0x18, 0x19, 0x1A, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x34, 0x35, 0x36, 0x37,
        0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4A, 0x53, 0x54, 0x55, 0x56,
        0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x73, 0x74, 0x75,
        0x76, 0x77, 0x78, 0x79, 0x7A, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8A, 0x92, 0x93,
        0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9,
        0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5, 0xC6,
        0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE1, 0xE2,
        0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7,
        0xF8, 0xF9, 0xFA, 0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00, 0xFB, 0xD5,
        0xDB, 0x20, 0xA8, 0xBA, 0xA3, 0xE8, 0xEB, 0xEC, 0x00, 0x3C, 0xF4, 0x76, 0x19, 0xE8, 0x78,
        0xAD, 0x99, 0xA0, 0x19, 0xE0, 0xD0, 0x6A, 0x40, 0x23, 0x9C, 0xD0, 0x07, 0xFF, 0xD9,
    ];

    #[test]
    fn test_get_image_metadata() {
        let meta = get_image_metadata(TINY_JPEG).unwrap();
        assert_eq!(meta.width, 1);
        assert_eq!(meta.height, 1);
        assert_eq!(meta.format, Some(ImageFormat::Jpeg));
    }

    #[test]
    fn test_needs_optimization_small_image() {
        let needs = needs_optimization(TINY_JPEG, 1568).unwrap();
        assert!(!needs);
    }

    #[test]
    fn test_resize_no_change_needed() {
        let result = resize_image(TINY_JPEG, 100, 100).unwrap();
        // Should return same data since no resize needed
        assert!(!result.is_empty());
    }

    #[test]
    fn test_optimize_for_llm_small_image() {
        let result = optimize_for_llm(TINY_JPEG, None).unwrap();
        assert!(!result.was_resized);
        assert_eq!(result.original_width, 1);
        assert_eq!(result.original_height, 1);
        assert_eq!(result.final_width, 1);
        assert_eq!(result.final_height, 1);
    }

    #[test]
    fn test_format_to_media_type() {
        assert_eq!(format_to_media_type(Some(ImageFormat::Jpeg)), "image/jpeg");
        assert_eq!(format_to_media_type(Some(ImageFormat::Png)), "image/png");
        assert_eq!(format_to_media_type(Some(ImageFormat::WebP)), "image/webp");
        assert_eq!(format_to_media_type(None), "image/jpeg");
    }
}
