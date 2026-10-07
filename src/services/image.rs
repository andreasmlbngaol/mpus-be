use crate::error::AppError;

/// Decode an uploaded image and re-encode it as lossy WebP.
///
/// Decoding through the `image` crate (rather than trusting the declared
/// content type) rejects anything that isn't a real image, and the dimension
/// limits guard against decompression bombs.
pub fn to_webp(bytes: &[u8], max_dimension: u32, quality: f32) -> Result<Vec<u8>, AppError> {
    let img = decode(bytes, max_dimension)?;
    encode_webp(&img, quality)
}

/// A small square WebP for map markers: center-crop to a square, then downscale.
pub fn thumbnail(bytes: &[u8], max_dim: u32, quality: f32) -> Result<Vec<u8>, AppError> {
    let img = decode(bytes, 8000)?;
    let square = center_square(&img);
    let resized = image::imageops::resize(&square, max_dim, max_dim, image::imageops::FilterType::Triangle);
    encode_webp(&resized, quality)
}

/// A square WebP for a stored sighting photo: center-crop to 1:1, then downscale only
/// if it's bigger than `max_dim`. Sightings are square everywhere in the app (Snap
/// preview, cat hero, map marker), so the stored file is square too — no more showing a
/// cropped preview while the full frame sits behind it.
pub fn square_webp(bytes: &[u8], max_dim: u32, quality: f32) -> Result<Vec<u8>, AppError> {
    let img = decode(bytes, 8000)?;
    let square = center_square(&img);
    let out = if square.width() > max_dim {
        image::imageops::resize(&square, max_dim, max_dim, image::imageops::FilterType::Triangle)
    } else {
        square
    };
    encode_webp(&out, quality)
}

/// The largest centered square that fits inside `img`.
fn center_square(img: &image::RgbImage) -> image::RgbImage {
    let (w, h) = (img.width(), img.height());
    let side = w.min(h).max(1);
    image::imageops::crop_imm(img, (w - side) / 2, (h - side) / 2, side, side).to_image()
}

/// Decode with dimension limits (guards against decompression bombs).
fn decode(bytes: &[u8], max_dimension: u32) -> Result<image::RgbImage, AppError> {
    use image::{ImageReader, Limits};

    let mut reader = ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| AppError::bad_request("That doesn't look like an image we recognize."))?;

    let mut limits = Limits::default();
    limits.max_image_width = Some(max_dimension);
    limits.max_image_height = Some(max_dimension);
    reader.limits(limits);

    let img = reader.decode().map_err(|_| {
        AppError::bad_request("That file isn't a valid image, or it's too large to handle.")
    })?;

    if img.width() == 0 || img.height() == 0 {
        return Err(AppError::bad_request("That image has no pixels to speak of."));
    }
    Ok(img.to_rgb8())
}

fn encode_webp(img: &image::RgbImage, quality: f32) -> Result<Vec<u8>, AppError> {
    let (w, h) = img.dimensions();
    let encoded = webp::Encoder::from_rgb(img.as_raw(), w, h).encode(quality);
    Ok(encoded.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_image() {
        assert!(to_webp(b"definitely not an image", 6000, 90.0).is_err());
    }

    #[test]
    fn encodes_png_to_webp() {
        use image::{ImageFormat, RgbaImage};
        use std::io::Cursor;

        let img = RgbaImage::from_pixel(2, 2, image::Rgba([220, 40, 40, 255]));
        let mut png = Vec::new();
        img.write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();

        let out = to_webp(&png, 6000, 90.0).unwrap();
        assert_eq!(&out[0..4], b"RIFF");
        assert_eq!(&out[8..12], b"WEBP");
    }

    #[test]
    fn thumbnail_is_square() {
        use image::{ImageFormat, RgbImage};
        use std::io::Cursor;

        let img = RgbImage::from_pixel(400, 200, image::Rgb([10, 200, 10]));
        let mut png = Vec::new();
        img.write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();

        let out = thumbnail(&png, 512, 90.0).unwrap();
        assert_eq!(&out[0..4], b"RIFF");
        assert_eq!(&out[8..12], b"WEBP");
    }

    #[test]
    fn square_webp_is_square_and_caps_size() {
        use image::{ImageFormat, RgbImage};
        use std::io::Cursor;

        // Wide frame: center-crops to a 400x400 square, under the 800 cap → kept as-is.
        let wide = RgbImage::from_pixel(1000, 400, image::Rgb([10, 200, 10]));
        let mut png = Vec::new();
        wide.write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();

        let out = square_webp(&png, 800, 90.0).unwrap();
        assert_eq!(&out[0..4], b"RIFF");
        assert_eq!(&out[8..12], b"WEBP");

        let decoded = decode(&out, 8000).unwrap();
        assert_eq!(decoded.width(), decoded.height(), "output must be 1:1");
        assert_eq!(decoded.width(), 400, "square is the short side, no upscale");

        // Big square: downscales the long side to the cap.
        let big = RgbImage::from_pixel(1000, 1000, image::Rgb([10, 200, 10]));
        let mut png = Vec::new();
        big.write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();

        let decoded = decode(&square_webp(&png, 800, 90.0).unwrap(), 8000).unwrap();
        assert_eq!(decoded.width(), 800, "long side downscales to max_dim");
        assert_eq!(decoded.height(), 800);
    }
}
