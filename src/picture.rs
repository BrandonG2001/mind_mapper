//! Pictures shown on nodes.
//!
//! A picture keeps the original PNG or JPEG file bytes, so saving never re-encodes it (or
//! loses quality), and it is stored inside the map file as a data URL, so a map stays a
//! single file. The bytes sit behind an `Arc`: copies of the map (undo history) share
//! them instead of duplicating megabytes per step.

use std::hash::{DefaultHasher, Hasher};
use std::io::Cursor;
use std::sync::Arc;

// `Engine` provides `encode`/`decode` on base64's engines; `as _` imports only the methods.
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use eframe::egui::{Color32, ColorImage, Vec2};
use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder as _, ImageFormat, ImageReader};
use thiserror::Error;

/// Largest picture file accepted, so maps stay quick to save and load.
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
/// Largest width or height accepted, in pixels, which bounds the memory decoding needs.
pub const MAX_SIDE: u32 = 8_000;
/// Largest size a picture is shown at on a node, in world units; smaller ones keep their
/// own size.
pub const MAX_DISPLAY: Vec2 = Vec2::new(200.0, 150.0);
/// Longest side of the copy decoded for the screen: sharp up to about 300% zoom.
const MAX_TEXTURE_SIDE: u32 = 600;

/// Errors that can occur while reading a picture.
#[derive(Debug, Error)]
pub enum PictureError {
    /// The data is not a PNG or JPEG file.
    #[error("only PNG and JPEG pictures can be used")]
    Unsupported,
    /// The file or its pixel size is over the limits.
    #[error("the picture is too large: at most 8 MB and 8000 pixels on a side")]
    TooLarge,
    /// The file could not be decoded or encoded.
    #[error("the picture could not be read: {0}")]
    Image(#[from] image::ImageError),
    /// A saved picture is not a PNG or JPEG data URL.
    #[error("the saved picture is not a valid data URL")]
    DataUrl,
}

/// The file formats a picture can have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PictureFormat {
    /// PNG: lossless, used for screenshots and pasted images.
    Png,
    /// JPEG: photos.
    Jpeg,
}

impl PictureFormat {
    /// The format whose signature `bytes` start with, if either.
    #[must_use]
    pub fn detect(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            Some(Self::Png)
        } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(Self::Jpeg)
        } else {
            None
        }
    }

    /// The format's media type, as used in data URLs.
    #[must_use]
    pub const fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
        }
    }

    const fn image_format(self) -> ImageFormat {
        match self {
            Self::Png => ImageFormat::Png,
            Self::Jpeg => ImageFormat::Jpeg,
        }
    }
}

/// A picture: the file's bytes, its format and its size in pixels.
#[derive(Debug, Clone)]
pub struct Picture {
    bytes: Arc<[u8]>,
    format: PictureFormat,
    width: u32,
    height: u32,
    /// A fingerprint of the bytes, so the screen copy can be cached per picture.
    key: u64,
}

impl PartialEq for Picture {
    fn eq(&self, other: &Self) -> bool {
        // Comparing fingerprints first avoids comparing megabytes in the common case.
        self.key == other.key && self.bytes == other.bytes
    }
}

impl Eq for Picture {}

impl Picture {
    /// Read a PNG or JPEG file's bytes.
    ///
    /// # Errors
    ///
    /// * `PictureError::Unsupported` if they are neither
    /// * `PictureError::TooLarge` if over [`MAX_BYTES`] or [`MAX_SIDE`]
    /// * `PictureError::Image` if the file is damaged
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, PictureError> {
        if bytes.len() > MAX_BYTES {
            return Err(PictureError::TooLarge);
        }
        let format = PictureFormat::detect(&bytes).ok_or(PictureError::Unsupported)?;
        // Reads only the header, not the pixels.
        let (width, height) = ImageReader::with_format(Cursor::new(&bytes), format.image_format())
            .into_dimensions()?;
        if width == 0 || height == 0 {
            return Err(PictureError::Unsupported);
        }
        if width > MAX_SIDE || height > MAX_SIDE {
            return Err(PictureError::TooLarge);
        }
        let mut hasher = DefaultHasher::new();
        hasher.write(&bytes);
        Ok(Self {
            key: hasher.finish(),
            bytes: bytes.into(),
            format,
            width,
            height,
        })
    }

    /// Store pixels (such as a pasted screenshot) as a PNG picture.
    ///
    /// # Errors
    ///
    /// As [`Picture::from_bytes`], or `PictureError::Image` if encoding fails.
    pub fn from_pixels(image: &ColorImage) -> Result<Self, PictureError> {
        let [width, height] = image.size;
        let width = u32::try_from(width).map_err(|_| PictureError::TooLarge)?;
        let height = u32::try_from(height).map_err(|_| PictureError::TooLarge)?;
        // egui keeps colours with alpha already multiplied in; PNG wants it separate.
        let rgba: Vec<u8> = image
            .pixels
            .iter()
            .flat_map(Color32::to_srgba_unmultiplied)
            .collect();
        let mut png = Vec::new();
        PngEncoder::new(&mut png).write_image(&rgba, width, height, ExtendedColorType::Rgba8)?;
        Self::from_bytes(png)
    }

    /// Read a picture saved by [`Picture::to_data_url`].
    ///
    /// # Errors
    ///
    /// `PictureError::DataUrl` if `url` is not a base64 PNG or JPEG data URL, else as
    /// [`Picture::from_bytes`].
    pub fn from_data_url(url: &str) -> Result<Self, PictureError> {
        let encoded = [PictureFormat::Png, PictureFormat::Jpeg]
            .into_iter()
            .find_map(|format| {
                url.strip_prefix("data:")?
                    .strip_prefix(format.mime())?
                    .strip_prefix(";base64,")
            })
            .ok_or(PictureError::DataUrl)?;
        let bytes = BASE64.decode(encoded).map_err(|_| PictureError::DataUrl)?;
        Self::from_bytes(bytes)
    }

    /// The picture as a data URL (`data:image/png;base64,...`), for map files and SVG.
    #[must_use]
    pub fn to_data_url(&self) -> String {
        format!(
            "data:{};base64,{}",
            self.format.mime(),
            BASE64.encode(&self.bytes)
        )
    }

    /// The original file bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The file format.
    #[must_use]
    pub const fn format(&self) -> PictureFormat {
        self.format
    }

    /// A fingerprint of the picture's bytes.
    #[must_use]
    pub const fn key(&self) -> u64 {
        self.key
    }

    /// This copy's share of the bytes in memory: copies of a map share them, so each
    /// counts only its part (for the undo history's memory budget).
    #[must_use]
    pub fn heap_share(&self) -> usize {
        self.bytes.len() / Arc::strong_count(&self.bytes)
    }

    /// The size the picture is shown at on a node, in world units: its own size in
    /// pixels, scaled down (never up) to fit within [`MAX_DISPLAY`].
    #[must_use]
    pub fn display_size(&self) -> Vec2 {
        // Both sides are at most MAX_SIDE, which fits in a u16.
        let to_f32 = |pixels: u32| f32::from(u16::try_from(pixels).unwrap_or(u16::MAX));
        let size = Vec2::new(to_f32(self.width), to_f32(self.height));
        let scale = (MAX_DISPLAY.x / size.x)
            .min(MAX_DISPLAY.y / size.y)
            .min(1.0);
        size * scale
    }

    /// Decode a copy for the screen, at most 600 pixels on a side (sharp up to about 300%
    /// zoom, without holding a full-size photo in video memory).
    ///
    /// # Errors
    ///
    /// Returns `PictureError::Image` if the file cannot be decoded.
    pub fn decode_for_screen(&self) -> Result<ColorImage, PictureError> {
        let mut image =
            image::load_from_memory_with_format(&self.bytes, self.format.image_format())?;
        if image.width() > MAX_TEXTURE_SIDE || image.height() > MAX_TEXTURE_SIDE {
            image = image.thumbnail(MAX_TEXTURE_SIDE, MAX_TEXTURE_SIDE);
        }
        let rgba = image.to_rgba8();
        let size = [rgba.width(), rgba.height()].map(|side| side as usize);
        Ok(ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
    }
}

/// A small solid picture, for tests.
#[cfg(test)]
pub(crate) fn test_picture(width: usize, height: usize) -> Picture {
    let color = Color32::from_rgb(200, 80, 40);
    let image = ColorImage::new([width, height], vec![color; width * height]);
    Picture::from_pixels(&image).expect("a small picture encodes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixels_become_a_png_that_round_trips_through_a_data_url() {
        let picture = test_picture(40, 20);

        assert_eq!(picture.format(), PictureFormat::Png);
        let url = picture.to_data_url();
        assert!(url.starts_with("data:image/png;base64,"));
        let back = Picture::from_data_url(&url).expect("valid");
        assert_eq!(back, picture);
        let decoded = back.decode_for_screen().expect("decodes");
        assert_eq!(decoded.size, [40, 20]);
        assert_eq!(decoded.pixels[0], Color32::from_rgb(200, 80, 40));
    }

    #[test]
    fn big_pictures_are_shown_smaller_and_small_ones_as_they_are() {
        assert_eq!(test_picture(40, 20).display_size(), Vec2::new(40.0, 20.0));
        let wide = test_picture(800, 200).display_size();
        assert!((wide.x - MAX_DISPLAY.x).abs() < 1e-3 && (wide.y - 50.0).abs() < 1e-3);
        let tall = test_picture(100, 600).display_size();
        assert!((tall.y - MAX_DISPLAY.y).abs() < 1e-3 && (tall.x - 25.0).abs() < 1e-3);
    }

    #[test]
    fn other_data_is_refused() {
        assert!(matches!(
            Picture::from_bytes(b"GIF89a....".to_vec()),
            Err(PictureError::Unsupported)
        ));
        assert!(matches!(
            Picture::from_bytes(b"\x89PNG\r\n\x1a\nbroken".to_vec()),
            Err(PictureError::Image(_))
        ));
        assert!(matches!(
            Picture::from_bytes(vec![0; MAX_BYTES + 1]),
            Err(PictureError::TooLarge)
        ));
        assert!(matches!(
            Picture::from_data_url("data:text/plain;base64,aGk="),
            Err(PictureError::DataUrl)
        ));
        assert_eq!(
            PictureFormat::detect(&[0xFF, 0xD8, 0xFF, 0xE0]),
            Some(PictureFormat::Jpeg)
        );
    }

    #[test]
    fn copies_share_their_bytes() {
        let picture = test_picture(10, 10);
        let alone = picture.heap_share();
        let copy = picture.clone();
        assert_eq!(copy.heap_share() * 2, alone);
    }
}
