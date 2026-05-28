use crate::wave::Wave;
use std::fs;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSourceStream, ReadOnlySource};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

const AUDIO_EXTS: &[&str] = &[
    "wav", "flac", "ogg", "mp3", "aac", "m4a", "alac", "aiff", "aif",
];

pub fn slot_root_dir() -> PathBuf {
    if let Some(data) = dirs::data_dir() {
        return data.join("MamplerSlots").join("slots");
    }

    if let Some(doc) = dirs::document_dir() {
        return doc.join("MamplerSlots").join("slots");
    }

    PathBuf::from("./MamplerSlots/slots")
}

pub fn ensure_slot_root() -> std::io::Result<PathBuf> {
    let dir = slot_root_dir();
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn slot_prefix(slot: usize) -> String {
    format!("slot_{:03}", slot.min(127))
}

pub fn find_slot_file(slot: usize) -> Option<PathBuf> {
    let root = slot_root_dir();
    let prefix = slot_prefix(slot);

    let entries = fs::read_dir(root).ok()?;

    let mut candidates = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();

        if !path.is_file() {
            continue;
        }

        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };

        let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
            continue;
        };

        if stem == prefix && is_supported_audio_ext(ext) {
            candidates.push(path);
        }
    }

    candidates.sort();
    candidates.into_iter().next()
}

pub fn slot_display_name(slot: usize) -> String {
    if let Some(path) = find_slot_file(slot) {
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("<file>");

        format!("{:03}: {}", slot.min(127), name)
    } else {
        format!("{:03}: Empty", slot.min(127))
    }
}

pub fn import_audio_to_slot_with_picker(slot: usize) -> Result<PathBuf, String> {
    let slot = slot.min(127);
    let root = ensure_slot_root().map_err(|e| e.to_string())?;

    let picked = pick_audio_file()?;

    let ext = picked
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("wav")
        .to_ascii_lowercase();

    let ext = if is_supported_audio_ext(&ext) {
        ext
    } else {
        String::from("wav")
    };

    remove_existing_slot_files(slot)?;

    let dest = root.join(format!("{}.{}", slot_prefix(slot), ext));

    copy_picked_file_to_path(&picked, &dest)?;

    Ok(dest)
}

pub fn load_slot(slot: usize) -> Result<Arc<Wave>, String> {
    let path = find_slot_file(slot)
        .ok_or_else(|| format!("No audio file in slot {}", slot.min(127)))?;

    load_wave_from_path(&path)
}

fn remove_existing_slot_files(slot: usize) -> Result<(), String> {
    let root = ensure_slot_root().map_err(|e| e.to_string())?;
    let prefix = slot_prefix(slot);

    for entry in fs::read_dir(root).map_err(|e| e.to_string())?.flatten() {
        let path = entry.path();

        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };

        if stem == prefix {
            let _ = fs::remove_file(path);
        }
    }

    Ok(())
}

fn is_supported_audio_ext(ext: &str) -> bool {
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    AUDIO_EXTS.iter().any(|e| *e == ext)
}

#[cfg(not(target_os = "android"))]
fn pick_audio_file() -> Result<PathBuf, String> {
    rlobkit_dialogs::blocking_open_file(
        "Pick audio file",
        &["wav", "flac", "ogg", "mp3", "aac", "m4a", "alac", "aiff", "aif"],
    )
    .ok_or_else(|| "No file picked".to_string())
}

#[cfg(target_os = "android")]
fn pick_audio_file() -> Result<PathBuf, String> {
    use rlobkit_dialogs::{RlobKit, RlobKitType};

    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|e| e.to_string())?;

    let file = runtime
        .block_on(async {
            RlobKit::open_single_file(RlobKitType::Custom {
                extensions: AUDIO_EXTS.iter().map(|s| s.to_string()).collect(),
                mime_types: vec![
                    "audio/*".to_string(),
                    "application/octet-stream".to_string(),
                ],
            })
            .await
        })
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "No file picked".to_string())?;

    let name = file
        .name()
        .unwrap_or_else(|| "picked_audio.wav".to_string());

    let ext = file
        .extension()
        .unwrap_or_else(|| "wav".to_string())
        .to_ascii_lowercase();

    let ext = if is_supported_audio_ext(&ext) {
        ext
    } else {
        "wav".to_string()
    };

    let temp = std::env::temp_dir().join(format!("mampler_picked_{}.{}", sanitize_name(&name), ext));

    RlobKit::read_file_to_path(&file, &temp)
        .map_err(|e| e.to_string())?;

    Ok(temp)
}

#[cfg(not(target_os = "android"))]
fn copy_picked_file_to_path(source: &Path, dest: &Path) -> Result<(), String> {
    fs::copy(source, dest)
        .map(|_| ())
        .map_err(|e| format!("Failed to copy '{}' to '{}': {}", source.display(), dest.display(), e))
}

#[cfg(target_os = "android")]
fn copy_picked_file_to_path(source: &Path, dest: &Path) -> Result<(), String> {
    fs::copy(source, dest)
        .map(|_| ())
        .map_err(|e| format!("Failed to copy '{}' to '{}': {}", source.display(), dest.display(), e))
}

#[cfg(target_os = "android")]
fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub fn load_wave_from_path(path: &Path) -> Result<Arc<Wave>, String> {
    let file = fs::File::open(path)
        .map_err(|e| format!("Cannot open '{}': {}", path.display(), e))?;

    let size = file.metadata().map(|m| m.len()).unwrap_or(0);

    let mss = MediaSourceStream::new(
        Box::new(ReadOnlySource::new(BufReader::new(file))),
        Default::default(),
    );

    let mut hint = Hint::new();

    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| {
            format!(
                "Cannot identify audio format '{}' ({} bytes): {}",
                path.display(),
                size,
                e
            )
        })?;

    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|track| track.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .ok_or_else(|| format!("No audio track in '{}'", path.display()))?;

    let track_id = track.id;
    let codec_params = track.codec_params.clone();

    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default())
        .map_err(|e| format!("No decoder for '{}': {}", path.display(), e))?;

    let mut output = Vec::<f32>::new();
    let mut sample_rate = codec_params.sample_rate.unwrap_or(0);

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(SymphoniaError::ResetRequired) => {
                continue;
            }
            Err(_) => break,
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(_) => continue,
        };

        let frames = decoded.frames();
        let spec = *decoded.spec();
        let channels = spec.channels.count().max(1);

        if sample_rate == 0 {
            sample_rate = spec.rate;
        }

        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);

        let samples = buffer.samples();
        let available_frames = samples.len() / channels;
        let frames = frames.min(available_frames);

        for frame in 0..frames {
            let base = frame * channels;

            if channels == 1 {
                let m = samples[base];
                output.push(m);
                output.push(m);
            } else {
                output.push(samples[base]);
                output.push(samples[base + 1]);
            }
        }
    }

    if output.is_empty() {
        return Err(format!("No audio decoded from '{}'", path.display()));
    }

    if sample_rate == 0 {
        return Err(format!("Unknown sample rate for '{}'", path.display()));
    }

    Ok(Arc::new(Wave::from_stereo(
        output,
        sample_rate,
        path.to_string_lossy().to_string(),
    )))
}
