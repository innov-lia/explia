//! Lecture des métadonnées multimédia : dimensions des images, durée et résolution des vidéos,
//! durée des sons. Seuls les en-têtes des fichiers sont lus (lecture seule, sans dépendance
//! supplémentaire) ; un format inconnu ou un fichier abîmé donne simplement « pas d'information ».

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Duration;

/// Informations lues dans un fichier multimédia.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MediaInfo {
    /// Largeur et hauteur : dimensions d'une image ou résolution d'une vidéo.
    pub dimensions: Option<(u32, u32)>,
    pub duration: Option<Duration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
}

/// Famille d'un fichier d'après son extension.
pub fn kind_of(path: &Path) -> Option<MediaKind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "bmp" | "gif" | "webp" => Some(MediaKind::Image),
        "mp4" | "m4v" | "mov" | "mkv" | "webm" | "avi" => Some(MediaKind::Video),
        "mp3" | "wav" | "flac" | "ogg" | "oga" | "opus" | "m4a" => Some(MediaKind::Audio),
        _ => None,
    }
}

/// Lit les informations d'un fichier ; ne panique jamais.
pub fn read_info(path: &Path) -> MediaInfo {
    let Some(kind) = kind_of(path) else {
        return MediaInfo::default();
    };
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if kind == MediaKind::Image {
        return MediaInfo {
            dimensions: image::image_dimensions(path).ok(),
            duration: None,
        };
    }
    let Ok(file) = File::open(path) else {
        return MediaInfo::default();
    };
    let mut reader = BufReader::new(file);
    read_from(&mut reader, &ext).unwrap_or_default()
}

/// Lit les informations depuis un lecteur, selon l'extension (en minuscules).
pub fn read_from<R: Read + Seek>(reader: &mut R, ext: &str) -> Option<MediaInfo> {
    match ext {
        "mp4" | "m4v" | "mov" | "m4a" => read_mp4(reader),
        "mkv" | "webm" => read_mkv(reader),
        "avi" => read_avi(reader),
        "wav" => read_wav(reader),
        "flac" => read_flac(reader),
        "mp3" => read_mp3(reader),
        "ogg" | "oga" | "opus" => read_ogg(reader),
        _ => None,
    }
}

fn seconds(value: f64) -> Option<Duration> {
    (value.is_finite() && value > 0.0 && value < 3.0e7).then(|| Duration::from_secs_f64(value))
}

fn file_len<R: Seek>(reader: &mut R) -> Option<u64> {
    let len = reader.seek(SeekFrom::End(0)).ok()?;
    reader.seek(SeekFrom::Start(0)).ok()?;
    Some(len)
}

fn read_bytes<R: Read>(reader: &mut R, n: usize) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; n];
    reader.read_exact(&mut buf).ok()?;
    Some(buf)
}

fn be_u32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn le_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

// ---------------------------------------------------------------------------------------------
// MP4 / MOV / M4A
// ---------------------------------------------------------------------------------------------

/// Taille maximale acceptée pour la boîte `moov` (métadonnées), en octets.
const MAX_MOOV_BYTES: u64 = 32 * 1024 * 1024;

fn read_mp4<R: Read + Seek>(reader: &mut R) -> Option<MediaInfo> {
    let len = file_len(reader)?;
    let mut pos = 0u64;
    for _ in 0..4096 {
        if pos + 8 > len {
            break;
        }
        reader.seek(SeekFrom::Start(pos)).ok()?;
        let head = read_bytes(reader, 8)?;
        let mut size = u64::from(be_u32(&head[0..4]));
        let mut header_len = 8u64;
        if size == 1 {
            let big = read_bytes(reader, 8)?;
            size = u64::from_be_bytes(big.try_into().ok()?);
            header_len = 16;
        } else if size == 0 {
            size = len - pos;
        }
        if size < header_len {
            break;
        }
        if &head[4..8] == b"moov" {
            let payload = size - header_len;
            if payload > MAX_MOOV_BYTES || pos + size > len {
                return None;
            }
            let body = read_bytes(reader, payload as usize)?;
            return Some(parse_moov(&body));
        }
        pos += size;
    }
    None
}

/// Sous-boîtes d'une boîte MP4 : `(type, contenu)`.
fn mp4_boxes(buf: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut boxes = Vec::new();
    let mut pos = 0usize;
    while pos + 8 <= buf.len() {
        let mut size = be_u32(&buf[pos..]) as usize;
        let kind = [buf[pos + 4], buf[pos + 5], buf[pos + 6], buf[pos + 7]];
        let mut header = 8usize;
        if size == 1 {
            if pos + 16 > buf.len() {
                break;
            }
            let big = u64::from_be_bytes(buf[pos + 8..pos + 16].try_into().unwrap_or([0; 8]));
            size = usize::try_from(big).unwrap_or(usize::MAX);
            header = 16;
        } else if size == 0 {
            size = buf.len() - pos;
        }
        if size < header || pos.saturating_add(size) > buf.len() {
            break;
        }
        boxes.push((kind, &buf[pos + header..pos + size]));
        pos += size;
    }
    boxes
}

fn parse_moov(moov: &[u8]) -> MediaInfo {
    let mut info = MediaInfo::default();
    for (kind, body) in mp4_boxes(moov) {
        match &kind {
            b"mvhd" => info.duration = mvhd_duration(body),
            b"trak" if info.dimensions.is_none() => {
                for (inner_kind, inner) in mp4_boxes(body) {
                    if &inner_kind == b"tkhd" {
                        info.dimensions = tkhd_dimensions(inner);
                    }
                }
            }
            _ => {}
        }
    }
    info
}

fn mvhd_duration(body: &[u8]) -> Option<Duration> {
    let (timescale, duration) = match body.first()? {
        0 if body.len() >= 20 => (
            u64::from(be_u32(&body[12..])),
            u64::from(be_u32(&body[16..])),
        ),
        1 if body.len() >= 32 => (
            u64::from(be_u32(&body[20..])),
            u64::from_be_bytes(body[24..32].try_into().ok()?),
        ),
        _ => return None,
    };
    if timescale == 0 || duration == 0 || duration == u64::from(u32::MAX) {
        return None;
    }
    seconds(duration as f64 / timescale as f64)
}

fn tkhd_dimensions(body: &[u8]) -> Option<(u32, u32)> {
    let offset = match body.first()? {
        0 => 76,
        1 => 88,
        _ => return None,
    };
    if body.len() < offset + 8 {
        return None;
    }
    // Largeur et hauteur sont des nombres à virgule fixe 16.16 : seule la partie entière compte.
    let width = be_u32(&body[offset..]) >> 16;
    let height = be_u32(&body[offset + 4..]) >> 16;
    (width > 0 && height > 0).then_some((width, height))
}

// ---------------------------------------------------------------------------------------------
// Matroska / WebM (EBML)
// ---------------------------------------------------------------------------------------------

const MKV_SEGMENT: u32 = 0x1853_8067;
const MKV_INFO: u32 = 0x1549_A966;
const MKV_TRACKS: u32 = 0x1654_AE6B;
const MKV_TRACK_ENTRY: u32 = 0xAE;
const MKV_VIDEO: u32 = 0xE0;
const MKV_CLUSTER: u32 = 0x1F43_B675;
const MKV_TIMECODE_SCALE: u32 = 0x2A_D7B1;
const MKV_DURATION: u32 = 0x4489;
const MKV_PIXEL_WIDTH: u32 = 0xB0;
const MKV_PIXEL_HEIGHT: u32 = 0xBA;

struct MkvState {
    /// Nanosecondes par unité de temps (1 000 000 par défaut).
    scale: u64,
    duration: Option<f64>,
    width: Option<u32>,
    height: Option<u32>,
}

/// Identifiant d'élément EBML (les bits de marquage sont conservés).
fn ebml_id<R: Read>(reader: &mut R) -> Option<u32> {
    let first = read_bytes(reader, 1)?[0];
    let len = first.leading_zeros() as usize + 1;
    if len > 4 {
        return None;
    }
    let mut value = u32::from(first);
    for byte in read_bytes(reader, len - 1)? {
        value = (value << 8) | u32::from(byte);
    }
    Some(value)
}

/// Taille d'un élément EBML : `Some(None)` si elle est inconnue (flux).
#[allow(clippy::option_option)]
fn ebml_size<R: Read>(reader: &mut R) -> Option<Option<u64>> {
    let first = read_bytes(reader, 1)?[0];
    if first == 0 {
        return None;
    }
    let len = first.leading_zeros() as usize + 1;
    // Pour une taille sur 8 octets, le masque est nul : calcul en 16 bits pour éviter un débordement.
    let mask = (0xFFu16 >> len.min(8)) as u8;
    let mut value = u64::from(first & mask);
    let mut all_ones = (first & mask) == mask;
    for byte in read_bytes(reader, len - 1)? {
        value = (value << 8) | u64::from(byte);
        all_ones &= byte == 0xFF;
    }
    Some((!all_ones).then_some(value))
}

fn ebml_uint<R: Read>(reader: &mut R, size: u64) -> Option<u64> {
    if size == 0 || size > 8 {
        return None;
    }
    Some(
        read_bytes(reader, size as usize)?
            .iter()
            .fold(0u64, |acc, b| (acc << 8) | u64::from(*b)),
    )
}

fn ebml_float<R: Read>(reader: &mut R, size: u64) -> Option<f64> {
    match size {
        4 => Some(f64::from(f32::from_be_bytes(
            read_bytes(reader, 4)?.try_into().ok()?,
        ))),
        8 => Some(f64::from_be_bytes(read_bytes(reader, 8)?.try_into().ok()?)),
        _ => None,
    }
}

fn mkv_walk<R: Read + Seek>(
    reader: &mut R,
    end: u64,
    depth: u8,
    state: &mut MkvState,
    budget: &mut u32,
) -> Option<()> {
    if depth > 6 {
        return Some(());
    }
    loop {
        let pos = reader.stream_position().ok()?;
        if pos >= end || *budget == 0 {
            return Some(());
        }
        *budget -= 1;
        let Some(id) = ebml_id(reader) else {
            return Some(());
        };
        let Some(size) = ebml_size(reader) else {
            return Some(());
        };
        let body_start = reader.stream_position().ok()?;
        let body_end = size.map_or(end, |s| body_start.saturating_add(s)).min(end);
        match id {
            MKV_SEGMENT | MKV_INFO | MKV_TRACKS | MKV_TRACK_ENTRY | MKV_VIDEO => {
                mkv_walk(reader, body_end, depth + 1, state, budget)?;
            }
            // Les données (clusters) suivent les en-têtes : inutile d'aller plus loin.
            MKV_CLUSTER => return Some(()),
            MKV_TIMECODE_SCALE => {
                if let Some(scale) = size.and_then(|s| ebml_uint(reader, s)) {
                    state.scale = scale;
                }
            }
            MKV_DURATION => state.duration = size.and_then(|s| ebml_float(reader, s)),
            MKV_PIXEL_WIDTH if state.width.is_none() => {
                state.width = size
                    .and_then(|s| ebml_uint(reader, s))
                    .and_then(|v| u32::try_from(v).ok());
            }
            MKV_PIXEL_HEIGHT if state.height.is_none() => {
                state.height = size
                    .and_then(|s| ebml_uint(reader, s))
                    .and_then(|v| u32::try_from(v).ok());
            }
            _ => {}
        }
        if size.is_none() {
            return Some(());
        }
        reader.seek(SeekFrom::Start(body_end)).ok()?;
    }
}

fn read_mkv<R: Read + Seek>(reader: &mut R) -> Option<MediaInfo> {
    let len = file_len(reader)?;
    let mut state = MkvState {
        scale: 1_000_000,
        duration: None,
        width: None,
        height: None,
    };
    let mut budget = 20_000u32;
    mkv_walk(reader, len, 0, &mut state, &mut budget)?;
    let duration = state
        .duration
        .and_then(|d| seconds(d * state.scale as f64 / 1.0e9));
    let dimensions = match (state.width, state.height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => Some((w, h)),
        _ => None,
    };
    (duration.is_some() || dimensions.is_some()).then_some(MediaInfo {
        dimensions,
        duration,
    })
}

// ---------------------------------------------------------------------------------------------
// AVI
// ---------------------------------------------------------------------------------------------

fn read_avi<R: Read + Seek>(reader: &mut R) -> Option<MediaInfo> {
    let len = file_len(reader)?;
    let head = read_bytes(reader, len.min(4096) as usize)?;
    if head.len() < 12 || &head[0..4] != b"RIFF" || &head[8..12] != b"AVI " {
        return None;
    }
    let at = head.windows(4).position(|w| w == b"avih")?;
    // « avih » + taille (4 octets), puis la structure MainAVIHeader.
    let header = head.get(at + 8..at + 8 + 40)?;
    let micros_per_frame = u64::from(le_u32(&header[0..]));
    let total_frames = u64::from(le_u32(&header[16..]));
    let (width, height) = (le_u32(&header[32..]), le_u32(&header[36..]));
    Some(MediaInfo {
        dimensions: (width > 0 && height > 0).then_some((width, height)),
        duration: seconds((micros_per_frame * total_frames) as f64 / 1.0e6),
    })
}

// ---------------------------------------------------------------------------------------------
// WAV
// ---------------------------------------------------------------------------------------------

fn read_wav<R: Read + Seek>(reader: &mut R) -> Option<MediaInfo> {
    let len = file_len(reader)?;
    let head = read_bytes(reader, 12)?;
    if &head[0..4] != b"RIFF" || &head[8..12] != b"WAVE" {
        return None;
    }
    let mut byte_rate = 0u64;
    let mut pos = 12u64;
    for _ in 0..64 {
        reader.seek(SeekFrom::Start(pos)).ok()?;
        let chunk = read_bytes(reader, 8)?;
        let size = u64::from(le_u32(&chunk[4..]));
        match &chunk[0..4] {
            b"fmt " if size >= 12 => {
                let fmt = read_bytes(reader, 12)?;
                byte_rate = u64::from(le_u32(&fmt[8..]));
            }
            b"data" => {
                if byte_rate == 0 {
                    return None;
                }
                // Taille inconnue ou incohérente (flux) : on prend le reste du fichier.
                let data = if size == 0xFFFF_FFFF || pos + 8 + size > len {
                    len.saturating_sub(pos + 8)
                } else {
                    size
                };
                return Some(MediaInfo {
                    dimensions: None,
                    duration: seconds(data as f64 / byte_rate as f64),
                });
            }
            _ => {}
        }
        pos += 8 + size + (size & 1);
    }
    None
}

// ---------------------------------------------------------------------------------------------
// FLAC
// ---------------------------------------------------------------------------------------------

fn read_flac<R: Read + Seek>(reader: &mut R) -> Option<MediaInfo> {
    let head = read_bytes(reader, 4 + 4 + 18)?;
    // « fLaC », puis un bloc de métadonnées de type 0 (STREAMINFO) de 34 octets.
    if &head[0..4] != b"fLaC" || head[4] & 0x7F != 0 {
        return None;
    }
    let info = &head[8..8 + 18];
    let b = &info[10..18];
    let sample_rate = (u64::from(b[0]) << 12) | (u64::from(b[1]) << 4) | (u64::from(b[2]) >> 4);
    let total_samples = (u64::from(b[3] & 0x0F) << 32) | u64::from(be_u32(&b[4..]));
    if sample_rate == 0 || total_samples == 0 {
        return None;
    }
    Some(MediaInfo {
        dimensions: None,
        duration: seconds(total_samples as f64 / sample_rate as f64),
    })
}

// ---------------------------------------------------------------------------------------------
// MP3
// ---------------------------------------------------------------------------------------------

const MP3_BITRATES_V1_L3: [u32; 15] = [
    0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
];
const MP3_BITRATES_V2_L3: [u32; 15] =
    [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];

fn read_mp3<R: Read + Seek>(reader: &mut R) -> Option<MediaInfo> {
    let len = file_len(reader)?;
    let head = read_bytes(reader, len.min(65_536) as usize)?;
    // Étiquette ID3v2 éventuelle au début.
    let mut start = 0usize;
    if head.len() >= 10 && &head[0..3] == b"ID3" {
        let size = (usize::from(head[6] & 0x7F) << 21)
            | (usize::from(head[7] & 0x7F) << 14)
            | (usize::from(head[8] & 0x7F) << 7)
            | usize::from(head[9] & 0x7F);
        start = 10 + size;
    }
    // Première trame : synchronisation sur 11 bits à 1.
    let frame = if start < head.len() {
        (start..head.len().saturating_sub(4))
            .find(|&i| head[i] == 0xFF && head[i + 1] & 0xE0 == 0xE0 && mp3_header_ok(&head[i..]))
    } else {
        // L'étiquette dépasse la zone lue : on lit la suite.
        None
    };
    let (frame_pos, header) = match frame {
        Some(i) => (i as u64, [head[i], head[i + 1], head[i + 2], head[i + 3]]),
        None => {
            reader.seek(SeekFrom::Start(start as u64)).ok()?;
            let more = read_bytes(reader, 8192.min(len.saturating_sub(start as u64)) as usize)?;
            let i = (0..more.len().saturating_sub(4)).find(|&i| {
                more[i] == 0xFF && more[i + 1] & 0xE0 == 0xE0 && mp3_header_ok(&more[i..])
            })?;
            (
                start as u64 + i as u64,
                [more[i], more[i + 1], more[i + 2], more[i + 3]],
            )
        }
    };
    let version = (header[1] >> 3) & 3; // 3 = MPEG 1, 2 = MPEG 2, 0 = MPEG 2.5
    let sample_rate_index = ((header[2] >> 2) & 3) as usize;
    let bitrate_index = ((header[2] >> 4) & 0x0F) as usize;
    let mono = (header[3] >> 6) == 3;
    let sample_rate = match (version, sample_rate_index) {
        (3, 0) => 44_100,
        (3, 1) => 48_000,
        (3, 2) => 32_000,
        (2, 0) => 22_050,
        (2, 1) => 24_000,
        (2, 2) => 16_000,
        (0, 0) => 11_025,
        (0, 1) => 12_000,
        (0, 2) => 8_000,
        _ => return None,
    };
    let (bitrates, samples_per_frame, side_info) = if version == 3 {
        (
            &MP3_BITRATES_V1_L3,
            1152u64,
            if mono { 17usize } else { 32 },
        )
    } else {
        (&MP3_BITRATES_V2_L3, 576u64, if mono { 9usize } else { 17 })
    };
    let bitrate = *bitrates.get(bitrate_index)? * 1000;
    if bitrate == 0 {
        return None;
    }
    // En-tête « Xing » ou « Info » (VBR) : contient le nombre de trames.
    reader
        .seek(SeekFrom::Start(frame_pos + 4 + side_info as u64))
        .ok()?;
    let tag = read_bytes(reader, 12).unwrap_or_default();
    if tag.len() == 12
        && (&tag[0..4] == b"Xing" || &tag[0..4] == b"Info")
        && be_u32(&tag[4..]) & 1 != 0
    {
        let frames = u64::from(be_u32(&tag[8..]));
        if frames > 0 {
            return Some(MediaInfo {
                dimensions: None,
                duration: seconds((frames * samples_per_frame) as f64 / f64::from(sample_rate)),
            });
        }
    }
    // Débit constant : durée estimée d'après la taille des données audio.
    let audio_bytes = len.saturating_sub(frame_pos);
    Some(MediaInfo {
        dimensions: None,
        duration: seconds(audio_bytes as f64 * 8.0 / f64::from(bitrate)),
    })
}

fn mp3_header_ok(bytes: &[u8]) -> bool {
    bytes.len() >= 4
        // Couche III uniquement, version valide, débit et fréquence valides.
        && (bytes[1] >> 1) & 3 == 1
        && (bytes[1] >> 3) & 3 != 1
        && (bytes[2] >> 4) & 0x0F != 0x0F
        && (bytes[2] >> 2) & 3 != 3
}

// ---------------------------------------------------------------------------------------------
// OGG (Vorbis et Opus)
// ---------------------------------------------------------------------------------------------

fn read_ogg<R: Read + Seek>(reader: &mut R) -> Option<MediaInfo> {
    let len = file_len(reader)?;
    let head = read_bytes(reader, len.min(512) as usize)?;
    if head.len() < 28 || &head[0..4] != b"OggS" {
        return None;
    }
    // Fréquence d'échantillonnage (et décalage initial pour Opus).
    let (rate, pre_skip) = if let Some(at) = head.windows(8).position(|w| w == b"OpusHead") {
        let pre_skip = u64::from(u16::from_le_bytes([
            *head.get(at + 10)?,
            *head.get(at + 11)?,
        ]));
        (48_000u64, pre_skip)
    } else {
        let at = head.windows(7).position(|w| w == b"\x01vorbis")?;
        (u64::from(le_u32(head.get(at + 12..at + 16)?)), 0)
    };
    if rate == 0 {
        return None;
    }
    // Dernière page : sa position absolue (« granule position ») donne le nombre d'échantillons.
    let tail_len = len.min(65_536);
    reader.seek(SeekFrom::Start(len - tail_len)).ok()?;
    let tail = read_bytes(reader, tail_len as usize)?;
    let at = tail.windows(4).rposition(|w| w == b"OggS")?;
    let granule = u64::from_le_bytes(tail.get(at + 6..at + 14)?.try_into().ok()?);
    if granule == 0 || granule == u64::MAX {
        return None;
    }
    Some(MediaInfo {
        dimensions: None,
        duration: seconds(granule.saturating_sub(pre_skip) as f64 / rate as f64),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn info(bytes: Vec<u8>, ext: &str) -> Option<MediaInfo> {
        read_from(&mut Cursor::new(bytes), ext)
    }

    fn secs(info: Option<MediaInfo>) -> f64 {
        info.and_then(|i| i.duration).expect("durée").as_secs_f64()
    }

    #[test]
    fn kinds_by_extension() {
        assert_eq!(kind_of(Path::new("a.PNG")), Some(MediaKind::Image));
        assert_eq!(kind_of(Path::new("a.mkv")), Some(MediaKind::Video));
        assert_eq!(kind_of(Path::new("a.flac")), Some(MediaKind::Audio));
        assert_eq!(kind_of(Path::new("a.txt")), None);
        assert_eq!(kind_of(Path::new("sans-extension")), None);
    }

    fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn mp4_duration_and_resolution() {
        let mut mvhd = vec![0u8; 4]; // version 0 + flags
        mvhd.extend_from_slice(&[0; 8]); // création, modification
        mvhd.extend_from_slice(&1000u32.to_be_bytes()); // échelle de temps
        mvhd.extend_from_slice(&90_000u32.to_be_bytes()); // durée : 90 s
        mvhd.extend_from_slice(&[0; 80]);
        let mut tkhd = vec![0u8; 76];
        tkhd.extend_from_slice(&(1920u32 << 16).to_be_bytes());
        tkhd.extend_from_slice(&(1080u32 << 16).to_be_bytes());
        let trak = mp4_box(b"trak", &mp4_box(b"tkhd", &tkhd));
        let moov = mp4_box(b"moov", &[mp4_box(b"mvhd", &mvhd), trak].concat());
        let file = [mp4_box(b"ftyp", b"isom\0\0\0\0"), moov].concat();
        let result = info(file, "mp4");
        assert_eq!(secs(result), 90.0);
        assert_eq!(result.unwrap().dimensions, Some((1920, 1080)));
    }

    #[test]
    fn mp4_without_moov_gives_nothing() {
        assert_eq!(info(mp4_box(b"ftyp", b"isom\0\0\0\0"), "mp4"), None);
        assert_eq!(info(b"n'importe quoi".to_vec(), "mp4"), None);
    }

    fn ebml(id: &[u8], body: &[u8]) -> Vec<u8> {
        let mut out = id.to_vec();
        // Taille sur 8 octets (0x01 + 7 octets) : simple à produire.
        out.push(0x01);
        out.extend_from_slice(&(body.len() as u64).to_be_bytes()[1..]);
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn mkv_duration_and_resolution() {
        let info_el = ebml(
            &[0x15, 0x49, 0xA9, 0x66],
            &[
                ebml(&[0x2A, 0xD7, 0xB1], &1_000_000u32.to_be_bytes()),
                ebml(&[0x44, 0x89], &120_000.0f64.to_be_bytes()),
            ]
            .concat(),
        );
        let video = ebml(
            &[0xE0],
            &[
                ebml(&[0xB0], &1280u16.to_be_bytes()),
                ebml(&[0xBA], &720u16.to_be_bytes()),
            ]
            .concat(),
        );
        let tracks = ebml(&[0x16, 0x54, 0xAE, 0x6B], &ebml(&[0xAE], &video));
        let segment = ebml(&[0x18, 0x53, 0x80, 0x67], &[info_el, tracks].concat());
        let file = [ebml(&[0x1A, 0x45, 0xDF, 0xA3], &[]), segment].concat();
        let result = info(file, "mkv");
        assert_eq!(secs(result), 120.0);
        assert_eq!(result.unwrap().dimensions, Some((1280, 720)));
    }

    #[test]
    fn wav_duration() {
        let mut file = b"RIFF\0\0\0\0WAVE".to_vec();
        file.extend_from_slice(b"fmt ");
        file.extend_from_slice(&16u32.to_le_bytes());
        file.extend_from_slice(&[1, 0, 2, 0]); // PCM, 2 canaux
        file.extend_from_slice(&44_100u32.to_le_bytes());
        file.extend_from_slice(&176_400u32.to_le_bytes()); // octets par seconde
        file.extend_from_slice(&[4, 0, 16, 0]);
        file.extend_from_slice(b"data");
        file.extend_from_slice(&352_800u32.to_le_bytes());
        file.extend_from_slice(&vec![0u8; 352_800]);
        assert_eq!(secs(info(file, "wav")), 2.0);
    }

    #[test]
    fn flac_duration() {
        let mut file = b"fLaC".to_vec();
        file.extend_from_slice(&[0x80, 0, 0, 34]); // dernier bloc, STREAMINFO, 34 octets
        let mut streaminfo = vec![0u8; 10];
        let sample_rate = 44_100u64;
        let total = 441_000u64; // 10 s
        let packed = (sample_rate << 44) | (1u64 << 41) | (15u64 << 36) | total;
        streaminfo.extend_from_slice(&packed.to_be_bytes());
        streaminfo.extend_from_slice(&[0; 16]);
        file.extend_from_slice(&streaminfo);
        assert_eq!(secs(info(file, "flac")), 10.0);
    }

    #[test]
    fn mp3_constant_bitrate_estimate() {
        // MPEG 1, couche III, 128 kb/s, 44,1 kHz, stéréo.
        let mut file = vec![0xFF, 0xFB, 0x90, 0x00];
        file.resize(160_000, 0); // 160 000 octets à 128 kb/s : 10 s
        assert!((secs(info(file, "mp3")) - 10.0).abs() < 0.01);
    }

    #[test]
    fn mp3_with_id3_tag_and_xing_header() {
        let mut file = b"ID3\x03\0\0\0\0\0\x0A".to_vec(); // étiquette de 10 octets
        file.extend_from_slice(&[0; 10]);
        let frame_start = file.len();
        file.extend_from_slice(&[0xFF, 0xFB, 0x90, 0x00]);
        file.resize(frame_start + 4 + 32, 0); // informations de la trame (stéréo, MPEG 1)
        file.extend_from_slice(b"Xing");
        file.extend_from_slice(&1u32.to_be_bytes()); // drapeaux : nombre de trames présent
        file.extend_from_slice(&383u32.to_be_bytes());
        file.resize(4_000, 0);
        // 383 trames de 1 152 échantillons à 44 100 Hz ≈ 10 s.
        assert!((secs(info(file, "mp3")) - 10.005).abs() < 0.01);
    }

    #[test]
    fn ogg_opus_duration() {
        let mut file = b"OggS".to_vec();
        file.resize(28, 0);
        file.extend_from_slice(b"OpusHead");
        file.extend_from_slice(&[1, 2]);
        file.extend_from_slice(&312u16.to_le_bytes()); // décalage initial
        file.resize(200, 0);
        let mut last_page = b"OggS\0\x04".to_vec();
        last_page.extend_from_slice(&(48_000u64 * 5 + 312).to_le_bytes());
        last_page.resize(40, 0);
        file.extend_from_slice(&last_page);
        assert!((secs(info(file, "opus")) - 5.0).abs() < 0.001);
    }

    #[test]
    fn avi_duration_and_resolution() {
        let mut file = b"RIFF\0\0\0\0AVI LIST\0\0\0\0hdrlavih".to_vec();
        file.extend_from_slice(&56u32.to_le_bytes());
        let mut header = vec![0u8; 56];
        header[0..4].copy_from_slice(&40_000u32.to_le_bytes()); // 25 images par seconde
        header[16..20].copy_from_slice(&250u32.to_le_bytes()); // 250 images : 10 s
        header[32..36].copy_from_slice(&640u32.to_le_bytes());
        header[36..40].copy_from_slice(&480u32.to_le_bytes());
        file.extend_from_slice(&header);
        let result = info(file, "avi");
        assert_eq!(secs(result), 10.0);
        assert_eq!(result.unwrap().dimensions, Some((640, 480)));
    }

    #[test]
    fn truncated_or_wrong_files_never_panic() {
        for ext in ["mp4", "mkv", "avi", "wav", "flac", "mp3", "ogg"] {
            assert_eq!(info(Vec::new(), ext), None, "{ext} vide");
            let junk: Vec<u8> = (0..300u32).map(|i| (i * 31 % 251) as u8).collect();
            let _ = info(junk, ext);
        }
        assert_eq!(info(vec![1, 2, 3], "txt"), None);
    }

    #[test]
    fn read_info_of_missing_or_unknown_files_is_empty() {
        assert_eq!(read_info(Path::new("absent.mp4")), MediaInfo::default());
        assert_eq!(read_info(Path::new("fichier.txt")), MediaInfo::default());
    }
}
