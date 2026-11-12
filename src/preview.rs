//! Génération des aperçus d'images et de fichiers texte (lecture seule).

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek};
use std::path::Path;

/// Au-delà de cette taille de fichier, on n'essaie pas de décoder l'image.
pub const MAX_IMAGE_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// Côté maximal de la miniature affichée.
pub const THUMBNAIL_SIZE: u32 = 1600;
/// Nombre maximal d'octets lus pour l'aperçu d'un fichier texte.
pub const MAX_TEXT_BYTES: usize = 64 * 1024;
/// Extensions des fichiers affichés comme du texte.
const TEXT_EXTENSIONS: &[&str] = &[
    "txt",
    "md",
    "log",
    "json",
    "csv",
    "xml",
    "html",
    "htm",
    "css",
    "js",
    "ts",
    "rs",
    "toml",
    "yaml",
    "yml",
    "ini",
    "cfg",
    "conf",
    "py",
    "c",
    "h",
    "cpp",
    "hpp",
    "cs",
    "java",
    "go",
    "sql",
    "bat",
    "cmd",
    "ps1",
    "sh",
    "gitignore",
];
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

/// Aperçu d'un fichier texte (début du fichier).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextPreview {
    pub content: String,
    /// `true` si le fichier est plus long que ce qui est affiché.
    pub truncated: bool,
}

/// Nombre maximal d'éléments listés pour une archive.
pub const MAX_ARCHIVE_ENTRIES: usize = 5000;

/// Un élément d'une archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveEntry {
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
}

/// Contenu d'une archive ZIP (liste seulement : rien n'est extrait).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivePreview {
    pub entries: Vec<ArchiveEntry>,
    /// Nombre total d'éléments dans l'archive (peut dépasser `entries.len()`).
    pub total: usize,
}

/// Contenu d'aperçu produit pour un fichier.
#[derive(Debug, Clone)]
pub enum PreviewContent {
    Image(ImagePreview),
    Text(TextPreview),
    Archive(ArchivePreview),
}

fn lowercase_extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
}

/// Formats d'image pris en charge pour l'aperçu : PNG, JPEG, BMP, GIF, WebP.
pub fn is_supported_image(path: &Path) -> bool {
    lowercase_extension(path).is_some_and(|ext| {
        matches!(
            ext.as_str(),
            "png" | "jpg" | "jpeg" | "bmp" | "gif" | "webp"
        )
    })
}

/// Fichiers texte courants, affichés comme du texte.
pub fn is_text_file(path: &Path) -> bool {
    lowercase_extension(path).is_some_and(|ext| TEXT_EXTENSIONS.contains(&ext.as_str()))
}

/// Archives ZIP dont on peut lister le contenu.
pub fn is_zip(path: &Path) -> bool {
    lowercase_extension(path).is_some_and(|ext| ext == "zip")
}

/// Vidéos courantes : ouvertes avec le lecteur par défaut de Windows.
pub fn is_video(path: &Path) -> bool {
    lowercase_extension(path).is_some_and(|ext| {
        matches!(
            ext.as_str(),
            "mp4" | "mkv" | "avi" | "mov" | "webm" | "wmv" | "m4v"
        )
    })
}

/// Vrai si un aperçu (image, texte ou contenu d'archive) peut être généré pour ce fichier.
pub fn is_previewable(path: &Path) -> bool {
    is_supported_image(path) || is_text_file(path) || is_zip(path)
}

/// Description du type d'un fichier d'après son extension.
pub fn kind_description(path: &Path) -> String {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) if is_supported_image(path) => format!("Image {}", ext.to_uppercase()),
        Some(ext) if is_text_file(path) => format!("Texte {}", ext.to_uppercase()),
        Some(ext) if is_video(path) => format!("Vidéo {}", ext.to_uppercase()),
        Some(ext) if is_zip(path) => format!("Archive {}", ext.to_uppercase()),
        Some(ext) => format!("Fichier {}", ext.to_uppercase()),
        None => "Fichier".to_string(),
    }
}

/// Convertit le début d'un fichier en texte ; refuse les fichiers binaires.
pub fn decode_text(bytes: &[u8], truncated: bool) -> Result<TextPreview, String> {
    if bytes.contains(&0) {
        return Err("Fichier binaire : pas d'aperçu texte".to_string());
    }
    Ok(TextPreview {
        content: String::from_utf8_lossy(bytes).into_owned(),
        truncated,
    })
}

/// Lit le début d'un fichier texte en lecture seule.
pub fn load_text_preview(path: &Path) -> Result<TextPreview, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    let mut bytes = Vec::new();
    file.take(MAX_TEXT_BYTES as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    decode_text(&bytes, len > MAX_TEXT_BYTES as u64)
}

/// Liste le contenu d'une archive ZIP sans rien extraire.
pub fn list_zip<R: Read + Seek>(reader: R) -> Result<ArchivePreview, String> {
    let mut archive = zip::ZipArchive::new(reader).map_err(|e| e.to_string())?;
    let total = archive.len();
    let mut entries = Vec::new();
    for index in 0..total.min(MAX_ARCHIVE_ENTRIES) {
        // `by_index_raw` lit seulement l'en-tête : pas de décompression, pas de mot de passe.
        let file = archive.by_index_raw(index).map_err(|e| e.to_string())?;
        entries.push(ArchiveEntry {
            name: file.name().map_err(|e| e.to_string())?.into_owned(),
            size: file.size(),
            is_dir: file.is_dir(),
        });
    }
    Ok(ArchivePreview { entries, total })
}

/// Ouvre un fichier ZIP en lecture seule et en liste le contenu.
pub fn load_zip_preview(path: &Path) -> Result<ArchivePreview, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    list_zip(BufReader::new(file))
}

/// Génère l'aperçu adapté au fichier (image, texte ou archive).
pub fn load_preview(path: &Path) -> Result<PreviewContent, String> {
    if is_supported_image(path) {
        load_image_preview(path).map(PreviewContent::Image)
    } else if is_text_file(path) {
        load_text_preview(path).map(PreviewContent::Text)
    } else if is_zip(path) {
        load_zip_preview(path).map(PreviewContent::Archive)
    } else {
        Err("Aperçu non pris en charge pour ce type de fichier".to_string())
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
        for name in [
            "a.png", "a.PNG", "a.jpg", "a.JPEG", "a.bmp", "a.gif", "a.WebP",
        ] {
            assert!(is_supported_image(Path::new(name)), "{name}");
        }
        for name in ["a.pdf", "a.txt", "a"] {
            assert!(!is_supported_image(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn text_files_are_recognized() {
        for name in ["a.txt", "a.MD", "main.rs", "a.json"] {
            assert!(is_text_file(Path::new(name)), "{name}");
            assert!(is_previewable(Path::new(name)), "{name}");
        }
        for name in ["a.exe", "a.pdf", "a.png", "a"] {
            assert!(!is_text_file(Path::new(name)), "{name}");
        }
        assert!(is_previewable(Path::new("a.png")));
        assert!(is_previewable(Path::new("a.ZIP")));
        assert!(!is_previewable(Path::new("a.exe")));
    }

    #[test]
    fn videos_are_recognized_but_not_previewed() {
        for name in ["film.mp4", "film.MKV", "a.avi"] {
            assert!(is_video(Path::new(name)), "{name}");
            assert!(!is_previewable(Path::new(name)), "{name}");
        }
        assert_eq!(kind_description(Path::new("film.mkv")), "Vidéo MKV");
    }

    fn sample_zip(files: usize) -> Vec<u8> {
        use std::io::Write;
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        writer.add_directory("docs/", options).unwrap();
        for i in 0..files {
            writer
                .start_file(format!("docs/fichier{i}.txt"), options)
                .unwrap();
            writer.write_all(b"bonjour").unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn lists_zip_content_without_extracting() {
        let preview = list_zip(Cursor::new(sample_zip(2))).unwrap();
        assert_eq!(preview.total, 3);
        assert_eq!(
            preview.entries[0],
            ArchiveEntry {
                name: "docs/".to_string(),
                size: 0,
                is_dir: true
            }
        );
        assert_eq!(preview.entries[1].name, "docs/fichier0.txt");
        assert_eq!(preview.entries[1].size, 7);
        assert!(!preview.entries[1].is_dir);
    }

    #[test]
    fn zip_listing_is_capped() {
        let preview = list_zip(Cursor::new(sample_zip(MAX_ARCHIVE_ENTRIES + 10))).unwrap();
        assert_eq!(preview.entries.len(), MAX_ARCHIVE_ENTRIES);
        assert_eq!(preview.total, MAX_ARCHIVE_ENTRIES + 11);
    }

    #[test]
    fn invalid_zip_is_an_error() {
        assert!(list_zip(Cursor::new(b"pas un zip".to_vec())).is_err());
    }

    #[test]
    fn decodes_text_and_rejects_binary() {
        let text = decode_text("héllo\nmonde".as_bytes(), false).unwrap();
        assert_eq!(text.content, "héllo\nmonde");
        assert!(!text.truncated);
        assert!(decode_text(&[b'a', 0, b'b'], false).is_err());
    }

    #[test]
    fn text_preview_of_a_real_file_is_truncated_when_long() {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let preview = load_text_preview(&manifest).unwrap();
        assert!(preview.content.contains("[package]"));
        let lock = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.lock");
        assert!(load_text_preview(&lock).unwrap().truncated);
    }

    #[test]
    fn load_preview_rejects_unsupported_types() {
        assert!(load_preview(Path::new("archive.zip")).is_err());
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
