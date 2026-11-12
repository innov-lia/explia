//! Génération des aperçus d'images (lecture seule).

use std::fs::File;
use std::io::{BufRead, BufReader, Seek};
use std::path::Path;

/// Au-delà de cette taille de fichier, on n'essaie pas de décoder l'image.
pub const MAX_IMAGE_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// Côté maximal de la miniature affichée.
pub const THUMBNAIL_SIZE: u32 = 512;
/// Mémoire maximale accordée au décodeur.
const MAX_DECODE_ALLOC: u64 = 512 * 1024 * 1024;

/// Miniature décodée, prête à devenir une texture.
#[derive(Debug, Clone)]
pub struct ImagePreview {
    /// Dimensions de l'image d'origine.
    pub original: (u32, u32),
    /// Dimensions de la miniature.
    pub size: [usize; 2],
    /// Pixels RGBA (non prémultipliés) de la miniature.
    pub rgba: Vec<u8>,
}

/// Formats d'image pris en charge pour l'aperçu : PNG, JPEG, BMP.
pub fn is_supported_image(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "bmp"
            )
        })
}

/// Description du type d'un fichier d'après son extension.
pub fn kind_description(path: &Path) -> String {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) if is_supported_image(path) => format!("Image {}", ext.to_uppercase()),
        Some(ext) => format!("Fichier {}", ext.to_uppercase()),
        None => "Fichier".to_string(),
    }
}

/// Décode une image depuis un lecteur et produit une miniature d'au plus `max_dim` pixels.
pub fn decode_preview<R: BufRead + Seek>(reader: R, max_dim: u32) -> Result<ImagePreview, String> {
    let mut reader = image::ImageReader::new(reader)
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    reader.limits(limits);
    let img = reader.decode().map_err(|e| e.to_string())?;
    let original = (img.width(), img.height());
    let thumb = if original.0 > max_dim || original.1 > max_dim {
        img.thumbnail(max_dim, max_dim)
    } else {
        img
    };
    let rgba = thumb.to_rgba8();
    Ok(ImagePreview {
        original,
        size: [rgba.width() as usize, rgba.height() as usize],
        rgba: rgba.into_raw(),
    })
}

/// Ouvre un fichier image en lecture seule et en génère l'aperçu.
pub fn load_image_preview(path: &Path) -> Result<ImagePreview, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    if len > MAX_IMAGE_FILE_BYTES {
        return Err("Fichier trop volumineux pour l'aperçu".to_string());
    }
    decode_preview(BufReader::new(file), THUMBNAIL_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageFormat, Rgba, RgbaImage};
    use std::io::Cursor;

    fn encode(format: ImageFormat, w: u32, h: u32) -> Vec<u8> {
        let img =
            image::DynamicImage::ImageRgba8(RgbaImage::from_pixel(w, h, Rgba([10, 20, 30, 255])));
        let mut bytes = Cursor::new(Vec::new());
        img.write_to(&mut bytes, format).unwrap();
        bytes.into_inner()
    }

    #[test]
    fn supported_extensions() {
        for name in ["a.png", "a.PNG", "a.jpg", "a.JPEG", "a.bmp"] {
            assert!(is_supported_image(Path::new(name)), "{name}");
        }
        for name in ["a.gif", "a.txt", "a"] {
            assert!(!is_supported_image(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn decodes_png_and_bmp_small_images_unchanged() {
        for format in [ImageFormat::Png, ImageFormat::Bmp] {
            let preview = decode_preview(Cursor::new(encode(format, 40, 20)), 512).unwrap();
            assert_eq!(preview.original, (40, 20));
            assert_eq!(preview.size, [40, 20]);
            assert_eq!(preview.rgba.len(), 40 * 20 * 4);
        }
    }

    #[test]
    fn decodes_jpeg() {
        let jpeg = encode(ImageFormat::Jpeg, 32, 32);
        let preview = decode_preview(Cursor::new(jpeg), 512).unwrap();
        assert_eq!(preview.original, (32, 32));
    }

    #[test]
    fn large_images_are_downscaled_keeping_aspect_ratio() {
        let preview = decode_preview(Cursor::new(encode(ImageFormat::Png, 400, 200)), 100).unwrap();
        assert_eq!(preview.original, (400, 200));
        assert_eq!(preview.size, [100, 50]);
    }

    #[test]
    fn invalid_data_is_an_error() {
        assert!(decode_preview(Cursor::new(b"pas une image".to_vec()), 512).is_err());
    }

    #[test]
    fn missing_file_is_an_error() {
        assert!(load_image_preview(Path::new("fichier-inexistant.png")).is_err());
    }
}
