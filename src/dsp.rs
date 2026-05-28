use core::f32::consts::PI;

#[inline]
pub fn fast_tanh(x: f32) -> f32 {
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

#[inline]
pub fn flush_denormals(x: f32) -> f32 {
    if x.abs() < 1e-24 {
        0.0
    } else {
        x
    }
}

#[inline]
pub fn db_to_linear(db: f32) -> f32 {
    10.0f32.powf(db / 20.0)
}

#[inline]
pub fn semitones_to_ratio(st: f32) -> f64 {
    2.0f64.powf(st as f64 / 12.0)
}

#[inline]
pub fn hermite(y0: f32, y1: f32, y2: f32, y3: f32, t: f32) -> f32 {
    let c0 = y1;
    let c1 = 0.5 * (y2 - y0);
    let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);

    ((c3 * t + c2) * t + c1) * t + c0
}

#[inline]
pub fn equal_power_pan(pan: f32) -> (f32, f32) {
    let x = (pan.clamp(-1.0, 1.0) + 1.0) * 0.5;
    let theta = x * core::f32::consts::FRAC_PI_2;
    (theta.cos(), theta.sin())
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FilterMode {
    Off,
    LowPass,
    HighPass,
    BandPass,
}

pub struct ZdfSvf {
    sr: f32,
    ic1eq: f32,
    ic2eq: f32,
    g: f32,
    r: f32,
    mode: FilterMode,
}

impl ZdfSvf {
    pub fn new(sr: f32) -> Self {
        Self {
            sr: sr.max(1.0),
            ic1eq: 0.0,
            ic2eq: 0.0,
            g: 0.0,
            r: 1.0,
            mode: FilterMode::Off,
        }
    }

    pub fn set_sample_rate(&mut self, sr: f32) {
        self.sr = sr.max(1.0);
    }

    pub fn reset(&mut self) {
        self.ic1eq = 0.0;
        self.ic2eq = 0.0;
    }

    #[inline]
    pub fn set(&mut self, cutoff_hz: f32, resonance: f32, mode: FilterMode) {
        self.mode = mode;

        if mode == FilterMode::Off {
            return;
        }

        let f = (cutoff_hz / self.sr).clamp(1e-5, 0.49);
        self.g = (PI * f).tan();
        self.r = (1.0 / resonance.max(0.05)).clamp(0.02, 10.0);
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        if self.mode == FilterMode::Off {
            return x;
        }

        let h = 1.0 / (1.0 + self.g * (self.g + self.r));
        let v1 = h * (self.ic1eq + self.g * (x - self.ic2eq));
        let v2 = self.ic2eq + self.g * v1;

        self.ic1eq = flush_denormals(2.0 * v1 - self.ic1eq);
        self.ic2eq = flush_denormals(2.0 * v2 - self.ic2eq);

        match self.mode {
            FilterMode::LowPass => v2,
            FilterMode::BandPass => v1,
            FilterMode::HighPass => x - self.r * v1 - v2,
            FilterMode::Off => x,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EnvState {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

pub struct Adsr {
    sr: f32,
    attack_ms: f32,
    decay_ms: f32,
    sustain: f32,
    release_ms: f32,
    level: f32,
    state: EnvState,
}

impl Adsr {
    pub fn new(sr: f32) -> Self {
        Self {
            sr: sr.max(1.0),
            attack_ms: 2.0,
            decay_ms: 100.0,
            sustain: 1.0,
            release_ms: 200.0,
            level: 0.0,
            state: EnvState::Idle,
        }
    }

    pub fn set_sample_rate(&mut self, sr: f32) {
        self.sr = sr.max(1.0);
    }

    pub fn set_ms(&mut self, attack_ms: f32, decay_ms: f32, sustain: f32, release_ms: f32) {
        self.attack_ms = attack_ms.max(0.0);
        self.decay_ms = decay_ms.max(0.0);
        self.sustain = sustain.clamp(0.0, 1.0);
        self.release_ms = release_ms.max(0.0);
    }

    pub fn note_on(&mut self) {
        self.state = EnvState::Attack;
    }

    pub fn note_off(&mut self) {
        if self.state != EnvState::Idle {
            self.state = EnvState::Release;
        }
    }

    pub fn reset(&mut self) {
        self.level = 0.0;
        self.state = EnvState::Idle;
    }

    #[inline]
    pub fn next(&mut self) -> f32 {
        match self.state {
            EnvState::Idle => {
                self.level = 0.0;
            }
            EnvState::Attack => {
                let samples = self.attack_ms * 0.001 * self.sr;
                let inc = if samples <= 1.0 { 1.0 } else { 1.0 / samples };
                self.level += inc;

                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.state = EnvState::Decay;
                }
            }
            EnvState::Decay => {
                let samples = self.decay_ms * 0.001 * self.sr;
                let dec = if samples <= 1.0 { 1.0 } else { 1.0 / samples };
                self.level -= dec;

                if self.level <= self.sustain {
                    self.level = self.sustain;
                    self.state = EnvState::Sustain;
                }
            }
            EnvState::Sustain => {}
            EnvState::Release => {
                let samples = self.release_ms * 0.001 * self.sr;
                let dec = if samples <= 1.0 { 1.0 } else { 1.0 / samples };
                self.level -= dec;

                if self.level <= 0.0 {
                    self.level = 0.0;
                    self.state = EnvState::Idle;
                }
            }
        }

        self.level.clamp(0.0, 1.0)
    }

    #[inline]
    pub fn is_idle(&self) -> bool {
        self.state == EnvState::Idle
    }

    #[inline]
    pub fn level(&self) -> f32 {
        self.level
    }
}

pub struct DecayEnv {
    sr: f32,
    value: f32,
    decay_ms: f32,
}

impl DecayEnv {
    pub fn new(sr: f32) -> Self {
        Self {
            sr: sr.max(1.0),
            value: 0.0,
            decay_ms: 50.0,
        }
    }

    pub fn set_sample_rate(&mut self, sr: f32) {
        self.sr = sr.max(1.0);
    }

    pub fn trigger(&mut self, decay_ms: f32) {
        self.value = 1.0;
        self.decay_ms = decay_ms.max(0.1);
    }

    #[inline]
    pub fn next(&mut self) -> f32 {
        let samples = self.decay_ms * 0.001 * self.sr;
        let coeff = (-1.0 / samples.max(1.0)).exp();
        self.value *= coeff;

        if self.value < 0.0001 {
            self.value = 0.0;
        }

        self.value
    }
}
