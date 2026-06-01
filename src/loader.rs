use crate::wave::Wave;
use std::fs;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use symphonia::core::codecs::CodecParameters;
use symphonia::core::codecs::audio::{AudioDecoderOptions, CODEC_ID_NULL_AUDIO};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::{MediaSourceStream, ReadOnlySource};
use symphonia::core::meta::MetadataOptions;

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

        if stem.starts_with(&prefix) && is_supported_audio_ext(ext) {
            candidates.push(path);
        }
    }

    candidates.sort();
    candidates.into_iter().next()
}

pub fn slot_display_name(slot: usize) -> String {
    if let Some(path) = find_slot_file(slot) {
        let prefix = slot_prefix(slot);
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("<file>");

        let display = if let Some(rest) = name.strip_prefix(&format!("{}_", prefix)) {
            rest.to_string()
        } else {
            name.to_string()
        };

        format!("{:03}: {}", slot.min(127), display)
    } else {
        format!("{:03}: Empty", slot.min(127))
    }
}

pub fn import_audio_to_slot_with_picker(slot: usize) -> Result<PathBuf, String> {
    let slot = slot.min(127);
    let root = ensure_slot_root().map_err(|e| e.to_string())?;

    let (picked, original_stem, is_temp) = pick_audio_file()?;

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

    let dest = root.join(format!("{}_{}.{}", slot_prefix(slot), original_stem, ext));

    let copy_result = fs::copy(&picked, &dest).map_err(|e| {
        format!(
            "Failed to copy '{}' to '{}': {}",
            picked.display(),
            dest.display(),
            e
        )
    });

    if is_temp {
        let _ = fs::remove_file(&picked);
    }

    copy_result.map(|_| dest)
}

pub fn load_slot(slot: usize) -> Result<Arc<Wave>, String> {
    let path =
        find_slot_file(slot).ok_or_else(|| format!("No audio file in slot {}", slot.min(127)))?;

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

        if stem.starts_with(&prefix) {
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
fn pick_audio_file() -> Result<(PathBuf, String, bool), String> {
    let path = rlobkit_dialogs::blocking_open_file(
        "Pick audio file",
        &[
            "wav", "flac", "ogg", "mp3", "aac", "m4a", "alac", "aiff", "aif",
        ],
    )
    .ok_or_else(|| "No file picked".to_string())?;

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("audio")
        .to_string();

    Ok((path, stem, false))
}

#[cfg(target_os = "android")]
fn pick_audio_file() -> Result<(PathBuf, String, bool), String> {
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

    let name = file.name();

    let ext = file
        .extension()
        .unwrap_or("wav")
        .to_ascii_lowercase();

    let ext = if is_supported_audio_ext(&ext) {
        ext
    } else {
        "wav".to_string()
    };

    let stem = name
        .rfind('.')
        .map(|pos| &name[..pos])
        .unwrap_or(&name)
        .to_string();

    let temp = std::env::temp_dir().join(format!(
        "mampler_picked_{}.{}",
        sanitize_name(&stem),
        ext
    ));

    RlobKit::read_file_to_path(&file, &temp).map_err(|e| e.to_string())?;

    Ok((temp, stem, true))
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
    let file =
        fs::File::open(path).map_err(|e| format!("Cannot open '{}': {}", path.display(), e))?;

    let size = file.metadata().map(|m| m.len()).unwrap_or(0);

    let mss = MediaSourceStream::new(
        Box::new(ReadOnlySource::new(BufReader::new(file))),
        Default::default(),
    );

    let mut hint = Hint::new();

    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
        hint.with_extension(ext);
    }

    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| {
            format!(
                "Cannot identify audio format '{}' ({} bytes): {}",
                path.display(),
                size,
                e
            )
        })?;

    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| format!("No audio track in '{}'", path.display()))?;

    let audio_params = match &track.codec_params {
        Some(CodecParameters::Audio(p)) => p,
        _ => return Err(format!("No audio codec parameters in '{}'", path.display())),
    };

    if audio_params.codec == CODEC_ID_NULL_AUDIO {
        return Err(format!("Null audio codec in '{}'", path.display()));
    }

    let track_id = track.id;

    let mut decoder = crate::codecs::get_codecs()
        .make_audio_decoder(audio_params, &AudioDecoderOptions::default())
        .map_err(|e| format!("No decoder for '{}': {}", path.display(), e))?;

    let mut output = Vec::<f32>::new();
    let mut sample_rate = audio_params.sample_rate.unwrap_or(0);

    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(SymphoniaError::ResetRequired) => continue,
            Err(_) => break,
        };

        if packet.track_id != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(_) => continue,
        };

        let spec = decoded.spec();
        let channels = spec.channels().count().max(1);

        if sample_rate == 0 {
            sample_rate = spec.rate();
        }

        let mut samples = Vec::<f32>::new();
        decoded.copy_to_vec_interleaved(&mut samples);

        let available_frames = samples.len() / channels;
        let frames = decoded.frames().min(available_frames);

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
