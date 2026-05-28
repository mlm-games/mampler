use crate::dsp::{
    Adsr, DecayEnv, FilterMode, ZdfSvf, db_to_linear, equal_power_pan,
    flush_denormals, semitones_to_ratio,
};
use crate::params::LoopMode;
use crate::wave::Wave;
use std::sync::Arc;

pub struct Voice {
    pub active: bool,
    pub note: u8,
    pub channel: u8,
    pub note_id: Option<i32>,
    pub key_down: bool,
    pub pending_sustain_release: bool,
    pub choke_group: u8,

    wave: Arc<Wave>,

    pos: f64,
    start: f64,
    end: f64,
    loop_start: f64,
    loop_end: f64,
    reverse: bool,
    one_shot: bool,
    loop_mode: LoopMode,

    target_rate: f64,
    current_rate: f64,
    glide_coeff: f64,

    velocity_gain: f32,
    region_gain: f32,
    pan_l: f32,
    pan_r: f32,
    normalize_gain: f32,

    pub env: Adsr,
    pitch_env: DecayEnv,
    pitch_env_amt_st: f32,

    filter_l: ZdfSvf,
    filter_r: ZdfSvf,

    pub age: u64,
}

impl Voice {
    pub fn new(sr: f32) -> Self {
        Self {
            active: false,
            note: 0,
            channel: 0,
            note_id: None,
            key_down: false,
            pending_sustain_release: false,
            choke_group: 0,

            wave: Arc::new(Wave::empty()),

            pos: 0.0,
            start: 0.0,
            end: 1.0,
            loop_start: 0.0,
            loop_end: 1.0,
            reverse: false,
            one_shot: false,
            loop_mode: LoopMode::Off,

            target_rate: 1.0,
            current_rate: 1.0,
            glide_coeff: 0.0,

            velocity_gain: 1.0,
            region_gain: 1.0,
            pan_l: 1.0,
            pan_r: 1.0,
            normalize_gain: 1.0,

            env: Adsr::new(sr),
            pitch_env: DecayEnv::new(sr),
            pitch_env_amt_st: 0.0,

            filter_l: ZdfSvf::new(sr),
            filter_r: ZdfSvf::new(sr),

            age: 0,
        }
    }

    pub fn set_sample_rate(&mut self, sr: f32) {
        self.env.set_sample_rate(sr);
        self.pitch_env.set_sample_rate(sr);
        self.filter_l.set_sample_rate(sr);
        self.filter_r.set_sample_rate(sr);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn start(
        &mut self,
        wave: Arc<Wave>,
        channel: u8,
        note: u8,
        note_id: Option<i32>,
        velocity_gain: f32,
        start: usize,
        end: usize,
        loop_start: usize,
        loop_end: usize,
        reverse: bool,
        one_shot: bool,
        loop_mode: LoopMode,
        rate: f64,
        previous_rate_for_glide: Option<f64>,
        glide_ms: f32,
        host_sr: f32,
        attack_ms: f32,
        decay_ms: f32,
        sustain: f32,
        release_ms: f32,
        pitch_env_amt_st: f32,
        pitch_env_decay_ms: f32,
        volume_db: f32,
        pan: f32,
        normalize: bool,
        choke_group: u8,
        age: u64,
    ) {
        self.active = true;
        self.wave = wave;
        self.channel = channel;
        self.note = note;
        self.note_id = note_id;
        self.key_down = true;
        self.pending_sustain_release = false;
        self.choke_group = choke_group;

        self.start = start as f64;
        self.end = end.max(start + 1) as f64;
        self.loop_start = loop_start as f64;
        self.loop_end = loop_end.max(loop_start + 1) as f64;
        self.reverse = reverse;
        self.one_shot = one_shot;
        self.loop_mode = loop_mode;

        self.pos = if reverse {
            self.end - 1.0
        } else {
            self.start
        };

        self.target_rate = if reverse { -rate.abs() } else { rate.abs() };

        self.current_rate = previous_rate_for_glide
            .unwrap_or(self.target_rate);

        self.glide_coeff = if glide_ms <= 0.0 {
            0.0
        } else {
            let samples = glide_ms * 0.001 * host_sr.max(1.0);
            1.0 - (-1.0 / samples.max(1.0)).exp() as f64
        };

        self.velocity_gain = velocity_gain;
        self.region_gain = db_to_linear(volume_db);
        let (pl, pr) = equal_power_pan(pan);
        self.pan_l = pl;
        self.pan_r = pr;
        self.normalize_gain = if normalize { self.wave.peak_gain } else { 1.0 };

        self.env.set_ms(attack_ms, decay_ms, sustain, release_ms);
        self.env.reset();
        self.env.note_on();

        self.pitch_env.trigger(pitch_env_decay_ms);
        self.pitch_env_amt_st = pitch_env_amt_st;

        self.filter_l.reset();
        self.filter_r.reset();

        self.age = age;
    }

    pub fn release(&mut self) {
        if self.active && !self.one_shot {
            self.key_down = false;
            self.pending_sustain_release = false;
            self.env.note_off();
        }
    }

    pub fn sustain_release_later(&mut self) {
        if self.active && !self.one_shot {
            self.key_down = false;
            self.pending_sustain_release = true;
        }
    }

    pub fn choke(&mut self) {
        self.stop();
    }

    pub fn stop(&mut self) {
        self.active = false;
        self.key_down = false;
        self.pending_sustain_release = false;
        self.env.reset();
    }

    pub fn render(
        &mut self,
        filter_mode: FilterMode,
        cutoff_hz: f32,
        resonance: f32,
        bend_st: f32,
        tune_st: f32,
    ) -> (f32, f32) {
        if !self.active || self.wave.is_empty() {
            return (0.0, 0.0);
        }

        self.handle_bounds();

        if !self.active {
            return (0.0, 0.0);
        }

        let env = self.env.next();

        if self.env.is_idle() {
            self.stop();
            return (0.0, 0.0);
        }

        if self.glide_coeff <= 0.0 {
            self.current_rate = self.target_rate;
        } else {
            self.current_rate += (self.target_rate - self.current_rate) * self.glide_coeff;
        }

        let pitch_env_st = self.pitch_env.next() * self.pitch_env_amt_st;
        let pitch_ratio = semitones_to_ratio(bend_st + tune_st + pitch_env_st);

        let (mut l, mut r) = self.wave.sample_stereo(self.pos);

        let amp = env
            * self.velocity_gain
            * self.region_gain
            * self.normalize_gain;

        l *= amp * self.pan_l;
        r *= amp * self.pan_r;

        self.filter_l.set(cutoff_hz, resonance, filter_mode);
        self.filter_r.set(cutoff_hz, resonance, filter_mode);

        l = self.filter_l.process(l);
        r = self.filter_r.process(r);

        self.pos += self.current_rate * pitch_ratio;

        (flush_denormals(l), flush_denormals(r))
    }

    pub fn current_rate(&self) -> f64 {
        self.current_rate
    }

    fn handle_bounds(&mut self) {
        let moving_forward = self.current_rate >= 0.0;

        let should_loop = match self.loop_mode {
            LoopMode::Off => false,
            LoopMode::Sustain => self.key_down || !self.pending_sustain_release,
            LoopMode::Continuous => true,
        };

        if moving_forward {
            if self.pos < self.start {
                self.pos = self.start;
            }

            if self.pos >= self.end {
                if should_loop {
                    self.wrap_forward();
                } else {
                    self.stop();
                }
            }
        } else {
            if self.pos >= self.end {
                self.pos = self.end - 1.0;
            }

            if self.pos < self.start {
                if should_loop {
                    self.wrap_reverse();
                } else {
                    self.stop();
                }
            }
        }
    }

    fn wrap_forward(&mut self) {
        let ls = self.loop_start.clamp(self.start, self.end - 1.0);
        let le = self.loop_end.clamp(ls + 1.0, self.end);
        let len = le - ls;

        if len <= 1.0 {
            self.pos = self.start;
        } else {
            self.pos = ls + (self.pos - le).rem_euclid(len);
        }
    }

    fn wrap_reverse(&mut self) {
        let ls = self.loop_start.clamp(self.start, self.end - 1.0);
        let le = self.loop_end.clamp(ls + 1.0, self.end);
        let len = le - ls;

        if len <= 1.0 {
            self.pos = self.end - 1.0;
        } else {
            self.pos = le - (ls - self.pos).rem_euclid(len);
        }
    }
}
