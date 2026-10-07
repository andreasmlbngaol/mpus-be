use std::sync::Mutex;

use image::imageops::FilterType;
use ort::session::{Session, builder::GraphOptimizationLevel};
use ort::value::Tensor;

use crate::{config::EmbedConfig, error::AppError};

/// ImageNet normalization, matching the MegaDescriptor-T preprocessing config.
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];
const SIZE: u32 = 224;
/// `crop_pct` from the model config: crop the central 90% before resizing.
const CROP_PCT: f32 = 0.9;
const EMBED_DIM: usize = 768;

/// Loads the ONNX model once and turns photos into 768-d embeddings.
///
/// `ort::Session::run` needs `&mut self`, so the session lives behind a `Mutex`.
/// Inference is CPU-bound and serialized here; callers run it inside
/// `spawn_blocking`. ponytail: single lock, one session — fine at this scale,
/// swap for a session pool if concurrent uploads ever queue up.
pub struct EmbeddingService {
    session: Mutex<Session>,
}

impl EmbeddingService {
    pub fn load(config: &EmbedConfig) -> Result<Self, AppError> {
        let session = Session::builder()
            .map_err(|e| AppError::internal(format!("ort init: {e}")))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| AppError::internal(format!("ort opt level: {e}")))?
            .with_intra_threads(config.threads)
            .map_err(|e| AppError::internal(format!("ort threads: {e}")))?
            // The CPU arena reserves memory up-front; off by default to keep the
            // small box's footprint low (a touch slower per call).
            .with_config_entry(
                "CPUExecutionProvider.use_arena",
                if config.memory_arena { "1" } else { "0" },
            )
            .map_err(|e| AppError::internal(format!("ort arena: {e}")))?
            .commit_from_file(&config.model_path)
            .map_err(|e| {
                AppError::internal(format!(
                    "couldn't load embedding model at {}: {e}",
                    config.model_path.display()
                ))
            })?;

        Ok(Self {
            session: Mutex::new(session),
        })
    }

    /// Decode an image and return its L2-normalized 768-d embedding.
    pub fn embed(&self, bytes: &[u8]) -> Result<Vec<f32>, AppError> {
        let input = preprocess(bytes)?;

        let tensor = Tensor::from_array(([1usize, 3, SIZE as usize, SIZE as usize], input))
            .map_err(|e| AppError::internal(format!("tensor build: {e}")))?;

        let mut session = self
            .session
            .lock()
            .map_err(|_| AppError::internal("embedding session poisoned"))?;

        let outputs = session
            .run(ort::inputs!["input" => tensor])
            .map_err(|e| AppError::internal(format!("inference failed: {e}")))?;

        let (_shape, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| AppError::internal(format!("bad model output: {e}")))?;

        if data.len() != DIMENSION {
            return Err(AppError::internal(format!(
                "model returned {} values, expected {DIMENSION}",
                data.len()
            )));
        }

        let mut embedding = data.to_vec();
        l2_normalize(&mut embedding);
        Ok(embedding)
    }
}

/// Decode + resize + crop + normalize into a flat NCHW `[3,224,224]` buffer.
fn preprocess(bytes: &[u8]) -> Result<Vec<f32>, AppError> {
    use image::{ImageReader, Limits};

    let mut reader = ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| AppError::bad_request("That doesn't look like an image we recognize."))?;

    let mut limits = Limits::default();
    limits.max_image_width = Some(8000);
    limits.max_image_height = Some(8000);
    reader.limits(limits);

    let img = reader
        .decode()
        .map_err(|_| AppError::bad_request("That file isn't a valid image, or it's too large to handle."))?
        .to_rgb8();

    // Center-crop to the central CROP_PCT, then resize to a square.
    let (w, h) = (img.width(), img.height());
    let cw = ((w as f32) * CROP_PCT) as u32;
    let ch = ((h as f32) * CROP_PCT) as u32;
    let cx = (w - cw) / 2;
    let cy = (h - ch) / 2;
    let cropped = image::imageops::crop_imm(&img, cx, cy, cw.max(1), ch.max(1)).to_image();
    let resized = image::imageops::resize(&cropped, SIZE, SIZE, FilterType::Triangle);

    // Flat CHW float buffer, normalized.
    let mut out = vec![0f32; 3 * SIZE as usize * SIZE as usize];
    let plane = (SIZE * SIZE) as usize;
    for (x, y, px) in resized.enumerate_pixels() {
        let idx = (y * SIZE + x) as usize;
        for c in 0..3 {
            let v = px.0[c] as f32 / 255.0;
            out[c * plane + idx] = (v - MEAN[c]) / STD[c];
        }
    }
    Ok(out)
}

/// Scale to unit length so cosine similarity == dot product.
fn l2_normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// The embedding dimension the DB column expects. Guards against a swapped model.
pub const DIMENSION: usize = EMBED_DIM;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l2_normalize_gives_unit_length() {
        let mut v = vec![3.0f32, 4.0];
        l2_normalize(&mut v);
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-6);
        assert!((v[0] - 0.6).abs() < 1e-6);
    }

    #[test]
    fn preprocess_shape_and_finiteness() {
        use image::{ImageFormat, RgbImage};
        use std::io::Cursor;

        let img = RgbImage::from_pixel(300, 200, image::Rgb([128, 128, 128]));
        let mut png = Vec::new();
        img.write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();

        let out = preprocess(&png).unwrap();
        assert_eq!(out.len(), 3 * 224 * 224);
        assert!(out.iter().all(|x| x.is_finite()));
    }

    /// Real-model check: the same image embeds to itself (cosine ~1), and two
    /// different images score lower. Needs `models/…int8.onnx` (gitignored), so
    /// it only runs on demand: `cargo test -- --ignored`.
    #[test]
    #[ignore = "requires the exported ONNX model in models/"]
    fn identical_images_are_closest() {
        use image::{ImageFormat, RgbImage};
        use std::io::Cursor;

        let cfg = EmbedConfig {
            model_path: std::path::PathBuf::from("./models/megadescriptor-t-224-int8.onnx"),
            threads: 1,
            memory_arena: false,
            top_k: 5,
            min_similarity: 0.55,
        };
        let svc = EmbeddingService::load(&cfg).unwrap();

        let make = |rgb: [u8; 3]| {
            let img = RgbImage::from_pixel(400, 400, image::Rgb(rgb));
            let mut png = Vec::new();
            img.write_to(&mut Cursor::new(&mut png), ImageFormat::Png).unwrap();
            png
        };

        let a = svc.embed(&make([200, 120, 40])).unwrap();
        let a2 = svc.embed(&make([200, 120, 40])).unwrap();
        let b = svc.embed(&make([40, 40, 200])).unwrap();

        let dot = |x: &[f32], y: &[f32]| x.iter().zip(y).map(|(a, b)| a * b).sum::<f32>();

        assert!(dot(&a, &a2) > 0.99, "same image should match itself");
        assert!(dot(&a, &b) < dot(&a, &a2), "different image should score lower");
    }
}
