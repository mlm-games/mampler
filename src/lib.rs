mod codecs;
mod dsp;
mod loader;
mod params;
mod voice;
mod wave;

use crossbeam::queue::ArrayQueue;
use nih_plug::prelude::*;
use params::{LoopMode, PlayMode, PlaybackMode, SamplerParams};
use std::num::NonZeroU32;
use std::sync::Arc;
use voice::Voice;
use wave::Wave;

const MAX_VOICES: usize = 128;

pub struct Mampler {
    params: Arc<SamplerParams>,

    sample_rate: f32,
    voices: Vec<Voice>,

    current_wave: Arc<Wave>,
    current_slot: usize,

    loader_messages: Arc<ArrayQueue<LoaderMessage>>,

    last_pick_trigger: bool,
    last_reload_trigger: bool,
    last_requested_slot: i32,

    frame_counter: u64,

    channels: [ChannelState; 16],
}

struct LoaderMessage {
    slot: usize,
    wave: Arc<Wave>,
}

#[derive(Clone)]
pub enum LoaderTask {
    PickToSlot(usize),
    LoadSlot(usize),
}

#[derive(Clone)]
struct ChannelState {
    sustain: bool,
    pitch_bend_norm: f32,
}

impl Default for ChannelState {
    fn default() -> Self {
        Self {
            sustain: false,
            pitch_bend_norm: 0.0,
        }
    }
}

impl Default for Mampler {
    fn default() -> Self {
        let sr = 44100.0;

        Self {
            params: Arc::new(SamplerParams::default()),

            sample_rate: sr,
            voices: (0..MAX_VOICES).map(|_| Voice::new(sr)).collect(),

            current_wave: Arc::new(Wave::empty()),
            current_slot: 0,

            loader_messages: Arc::new(ArrayQueue::new(8)),

            last_pick_trigger: false,
            last_reload_trigger: false,
            last_requested_slot: -1,

            frame_counter: 0,

            channels: std::array::from_fn(|_| ChannelState::default()),
        }
    }
}

impl Plugin for Mampler {
    const NAME: &'static str = "Mampler";
    const VENDOR: &'static str = "mlm-games";
    const URL: &'static str = "https://github.com/mlm-games";
    const EMAIL: &'static str = "me@example.com";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        aux_input_ports: &[],
        aux_output_ports: &[],
        names: PortNames::const_default(),
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::Basic;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = LoaderTask;

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        ctx: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate.max(1.0);

        for voice in &mut self.voices {
            voice.set_sample_rate(self.sample_rate);
        }

        let slot = self.params.slot.value().clamp(0, 127) as usize;
        ctx.execute(LoaderTask::LoadSlot(slot));
        self.last_requested_slot = slot as i32;

        true
    }

    fn reset(&mut self) {
        self.frame_counter = 0;
        self.channels = std::array::from_fn(|_| ChannelState::default());

        for voice in &mut self.voices {
            voice.stop();
        }
    }

    fn process(
        &mut self,
        buffer: &mut Buffer<'_>,
        _aux: &mut AuxiliaryBuffers<'_>,
        ctx: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        self.drain_loader_messages();

        let slot = self.params.slot.value().clamp(0, 127);

        if slot != self.last_requested_slot {
            self.last_requested_slot = slot;
            ctx.execute_background(LoaderTask::LoadSlot(slot as usize));
        }

        let pick = self.params.pick_to_slot.value();
        if pick && !self.last_pick_trigger {
            ctx.execute_background(LoaderTask::PickToSlot(slot as usize));
        }
        self.last_pick_trigger = pick;

        let reload = self.params.reload_slot.value();
        if reload && !self.last_reload_trigger {
            ctx.execute_background(LoaderTask::LoadSlot(slot as usize));
        }
        self.last_reload_trigger = reload;

        let max_voices = self.params.max_voices.value().clamp(1, MAX_VOICES as i32) as usize;
        let filter_mode = self.params.filter.value().to_dsp();
        let cutoff = self.params.cutoff_hz.value();
        let resonance = self.params.resonance.value();
        let gain = self.params.gain.value();
        let pan = self.params.pan.value();
        let tune = self.params.tune_st.value();
        let bend_range = self.params.bend_range_st.value() as f32;

        let mut next_event = ctx.next_event();

        for (sample_idx, mut frame) in buffer.iter_samples().enumerate() {
            while let Some(event) = next_event {
                if event.timing() > sample_idx as u32 {
                    break;
                }

                self.handle_event(event, max_voices);

                next_event = ctx.next_event();
            }

            let mut out_l = 0.0f32;
            let mut out_r = 0.0f32;

            for voice in &mut self.voices[..max_voices] {
                if !voice.active {
                    continue;
                }

                let ch = (voice.channel as usize).min(15);
                let bend_norm = self.channels[ch].pitch_bend_norm;
                let bend_st = bend_norm * bend_range;

                let (l, r) = voice.render(
                    filter_mode,
                    cutoff,
                    resonance,
                    bend_st,
                    tune,
                );

                out_l += l;
                out_r += r;
            }

            let (pan_l, pan_r) = dsp::equal_power_pan(pan);
            out_l *= gain * pan_l;
            out_r *= gain * pan_r;

            out_l = dsp::fast_tanh(out_l);
            out_r = dsp::fast_tanh(out_r);

            let mut channels = frame.iter_mut();

            if let Some(sample) = channels.next() {
                *sample = out_l;
            }

            if let Some(sample) = channels.next() {
                *sample = out_r;
            }

            self.frame_counter = self.frame_counter.wrapping_add(1);
        }

        ProcessStatus::Normal
    }

    fn task_executor(&mut self) -> TaskExecutor<Self> {
        let tx = self.loader_messages.clone();

        Box::new(move |task| match task {
            LoaderTask::PickToSlot(slot) => {
                match loader::import_audio_to_slot_with_picker(slot)
                    .and_then(|_| loader::load_slot(slot))
                {
                    Ok(wave) => {
                        let _ = tx.push(LoaderMessage { slot, wave });
                    }
                    Err(err) => {
                        nih_plug::nih_log!("Mampler: pick/load failed: {}", err);
                    }
                }
            }
            LoaderTask::LoadSlot(slot) => {
                match loader::load_slot(slot) {
                    Ok(wave) => {
                        let _ = tx.push(LoaderMessage { slot, wave });
                    }
                    Err(err) => {
                        nih_plug::nih_log!("Mampler: slot {} empty/unloadable: {}", slot, err);
                    }
                }
            }
        })
    }
}

impl ClapPlugin for Mampler {
    const CLAP_ID: &'static str = "dev.mlm-games.mampler";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Non-UI TX16Wx-style sampler with persistent slots and rlobkit import");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;

    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Sampler,
        ClapFeature::Stereo,
    ];

    fn remote_controls(&self, _context: &mut impl RemoteControlsContext) {}
}

nih_export_clap!(Mampler);

impl Mampler {
    fn drain_loader_messages(&mut self) {
        while let Some(msg) = self.loader_messages.pop() {
            self.current_slot = msg.slot;
            self.current_wave = msg.wave;

            for voice in &mut self.voices {
                voice.stop();
            }

            nih_plug::nih_log!(
                "Mampler: loaded slot {}: {}",
                self.current_slot,
                self.current_wave.path
            );
        }
    }

    fn handle_event(&mut self, event: NoteEvent<()>, max_voices: usize) {
        match event {
            NoteEvent::NoteOn {
                channel,
                note,
                velocity,
                voice_id,
                ..
            } => {
                if velocity <= 0.0 {
                    self.note_off(channel, note, voice_id);
                } else {
                    self.note_on(channel, note, velocity, voice_id, max_voices);
                }
            }

            NoteEvent::NoteOff {
                channel,
                note,
                voice_id,
                ..
            } => {
                self.note_off(channel, note, voice_id);
            }

            NoteEvent::MidiCC {
                channel,
                cc,
                value,
                ..
            } => {
                self.handle_cc(channel, cc, value);
            }

            NoteEvent::MidiPitchBend {
                channel,
                value,
                ..
            } => {
                let ch = (channel as usize).min(15);

                let centered = if (0.0..=1.0).contains(&value) {
                    (value - 0.5) * 2.0
                } else {
                    value
                };

                self.channels[ch].pitch_bend_norm = centered.clamp(-1.0, 1.0);
            }

            NoteEvent::Choke {
                channel,
                note,
                voice_id,
                ..
            } => {
                for voice in &mut self.voices {
                    if voice.active
                        && voice.channel == channel
                        && voice.note == note
                        && (voice_id.is_none() || voice.note_id == voice_id)
                    {
                        voice.choke();
                    }
                }
            }

            _ => {}
        }
    }

    fn note_on(
        &mut self,
        channel: u8,
        note: u8,
        velocity: f32,
        voice_id: Option<i32>,
        max_voices: usize,
    ) {
        if self.current_wave.is_empty() {
            return;
        }

        let mode = self.params.mode.value();
        let play_mode = self.params.play_mode.value();
        let choke_group = self.params.choke_group.value().clamp(0, 16) as u8;

        if choke_group > 0 {
            for voice in &mut self.voices {
                if voice.active && voice.choke_group == choke_group {
                    voice.choke();
                }
            }
        }

        match play_mode {
            PlayMode::Poly => {}
            PlayMode::Mono => {
                for voice in &mut self.voices {
                    if voice.active && voice.channel == channel {
                        voice.release();
                    }
                }
            }
            PlayMode::Legato => {}
        }

        let Some(region) = self.compute_region_for_note(note) else {
            return;
        };

        let root = self.params.root_note.value().clamp(0, 127) as u8;

        let pitch_by_key = match mode {
            PlaybackMode::Classic | PlaybackMode::Gate | PlaybackMode::OneShot => {
                note as f32 - root as f32
            }
            PlaybackMode::SliceGrid | PlaybackMode::SliceTransient => 0.0,
        };

        let sample_rate_ratio = self.current_wave.sample_rate as f64 / self.sample_rate as f64;
        let rate = dsp::semitones_to_ratio(pitch_by_key) * sample_rate_ratio;

        let vel_sens = self.params.vel_sens.value();
        let vel_curve = self.params.vel_curve.value().max(0.001);
        let vel = velocity.clamp(0.0, 1.0).powf(vel_curve);
        let velocity_gain = 1.0 - vel_sens + vel_sens * vel;

        let mut previous_rate = None;
        let mut target_slot = None;

        if play_mode == PlayMode::Legato {
            for (idx, voice) in self.voices[..max_voices].iter().enumerate() {
                if voice.active && voice.channel == channel && voice.key_down {
                    previous_rate = Some(voice.current_rate());
                    target_slot = Some(idx);
                    break;
                }
            }
        }

        let slot = target_slot.unwrap_or_else(|| self.alloc_voice(max_voices));

        let one_shot = mode == PlaybackMode::OneShot;

        self.voices[slot].start(
            self.current_wave.clone(),
            channel,
            note,
            voice_id,
            velocity_gain,
            region.start,
            region.end,
            region.loop_start,
            region.loop_end,
            self.params.reverse.value(),
            one_shot,
            if one_shot { LoopMode::Off } else { self.params.loop_mode.value() },
            rate,
            previous_rate,
            self.params.glide_ms.value(),
            self.sample_rate,
            self.params.attack_ms.value(),
            self.params.decay_ms.value(),
            self.params.sustain.value(),
            self.params.release_ms.value(),
            self.params.pitch_env_amt_st.value(),
            self.params.pitch_env_decay_ms.value(),
            0.0,
            0.0,
            self.params.normalize.value(),
            choke_group,
            self.frame_counter,
        );
    }

    fn note_off(&mut self, channel: u8, note: u8, voice_id: Option<i32>) {
        let ch = (channel as usize).min(15);
        let sustain = self.channels[ch].sustain;

        for voice in &mut self.voices {
            if !voice.active {
                continue;
            }

            if voice.channel != channel || voice.note != note {
                continue;
            }

            if voice_id.is_some() && voice.note_id != voice_id {
                continue;
            }

            if sustain {
                voice.sustain_release_later();
            } else {
                voice.release();
            }
        }
    }

    fn handle_cc(&mut self, channel: u8, cc: u8, value: f32) {
        let ch = (channel as usize).min(15);

        match cc {
            64 => {
                let was_down = self.channels[ch].sustain;
                let is_down = value >= 0.5;

                self.channels[ch].sustain = is_down;

                if was_down && !is_down {
                    for voice in &mut self.voices {
                        if voice.active && voice.channel == channel && voice.pending_sustain_release {
                            voice.release();
                        }
                    }
                }
            }

            120 | 123 => {
                for voice in &mut self.voices {
                    if voice.active && voice.channel == channel {
                        voice.release();
                    }
                }
            }

            _ => {}
        }
    }

    fn alloc_voice(&mut self, max_voices: usize) -> usize {
        let limit = max_voices.clamp(1, self.voices.len());

        if let Some(idx) = self.voices[..limit].iter().position(|v| !v.active) {
            return idx;
        }

        self.voices[..limit]
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                let sa = a.env.level() + if a.key_down { 1.0 } else { 0.0 };
                let sb = b.env.level() + if b.key_down { 1.0 } else { 0.0 };

                sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(idx, _)| idx)
            .unwrap_or(0)
    }

    fn compute_region_for_note(&self, note: u8) -> Option<ComputedRegion> {
        let wave = &self.current_wave;
        let frames = wave.num_frames;

        if frames == 0 {
            return None;
        }

        let start_pct = self.params.start_pct.value().clamp(0.0, 100.0) * 0.01;
        let end_pct = self.params.end_pct.value().clamp(0.0, 100.0) * 0.01;

        let mut start = (frames as f32 * start_pct) as usize;
        let mut end = (frames as f32 * end_pct) as usize;

        if end <= start {
            end = (start + 1).min(frames);
        }

        start = start.min(frames - 1);
        end = end.clamp(start + 1, frames);

        if self.params.snap_zero.value() {
            start = wave.snap_to_zero(start);
            end = wave.snap_to_zero(end).max(start + 1).min(frames);
        }

        match self.params.mode.value() {
            PlaybackMode::Classic | PlaybackMode::Gate | PlaybackMode::OneShot => {
                Some(self.make_region(start, end))
            }

            PlaybackMode::SliceGrid => {
                let base = self.params.slice_base_note.value().clamp(0, 127) as u8;
                if note < base {
                    return None;
                }

                let idx = (note - base) as usize;
                let count = self.params.slice_count.value().clamp(1, 64) as usize;

                if idx >= count {
                    return None;
                }

                let len = end - start;
                let slice_start = start + (len * idx) / count;
                let slice_end = if idx + 1 >= count {
                    end
                } else {
                    start + (len * (idx + 1)) / count
                };

                Some(self.make_region(slice_start, slice_end.max(slice_start + 1)))
            }

            PlaybackMode::SliceTransient => {
                let base = self.params.slice_base_note.value().clamp(0, 127) as u8;
                if note < base {
                    return None;
                }

                let idx = (note - base) as usize;

                let mut points = Vec::new();
                points.push(start);

                for &t in &wave.transients {
                    if t > start && t < end {
                        points.push(t);
                    }
                }

                points.push(end);
                points.sort_unstable();
                points.dedup();

                if idx + 1 >= points.len() {
                    return None;
                }

                Some(self.make_region(points[idx], points[idx + 1].max(points[idx] + 1)))
            }
        }
    }

    fn make_region(&self, start: usize, end: usize) -> ComputedRegion {
        let len = end.saturating_sub(start).max(1);

        let ls_pct = self.params.loop_start_pct.value().clamp(0.0, 100.0) * 0.01;
        let le_pct = self.params.loop_end_pct.value().clamp(0.0, 100.0) * 0.01;

        let mut loop_start = start + (len as f32 * ls_pct) as usize;
        let mut loop_end = start + (len as f32 * le_pct) as usize;

        loop_start = loop_start.clamp(start, end - 1);
        loop_end = loop_end.clamp(loop_start + 1, end);

        if self.params.snap_zero.value() {
            loop_start = self.current_wave.snap_to_zero(loop_start).clamp(start, end - 1);
            loop_end = self.current_wave.snap_to_zero(loop_end).clamp(loop_start + 1, end);
        }

        ComputedRegion {
            start,
            end,
            loop_start,
            loop_end,
        }
    }
}

struct ComputedRegion {
    start: usize,
    end: usize,
    loop_start: usize,
    loop_end: usize,
}

pub fn slot_name_for_index(idx: i32) -> String {
    loader::slot_display_name(idx.clamp(0, 127) as usize)
}
