use nih_plug::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq, Enum)]
pub enum PlaybackMode {
    Classic,
    Gate,
    OneShot,
    SliceGrid,
    SliceTransient,
}

#[derive(Clone, Copy, PartialEq, Eq, Enum)]
pub enum LoopMode {
    Off,
    Sustain,
    Continuous,
}

#[derive(Clone, Copy, PartialEq, Eq, Enum)]
pub enum PlayMode {
    Poly,
    Mono,
    Legato,
}

#[derive(Clone, Copy, PartialEq, Eq, Enum)]
pub enum FilterKind {
    Off,
    LowPass,
    HighPass,
    BandPass,
}

impl FilterKind {
    pub fn to_dsp(self) -> crate::dsp::FilterMode {
        match self {
            Self::Off => crate::dsp::FilterMode::Off,
            Self::LowPass => crate::dsp::FilterMode::LowPass,
            Self::HighPass => crate::dsp::FilterMode::HighPass,
            Self::BandPass => crate::dsp::FilterMode::BandPass,
        }
    }
}

#[derive(Params)]
pub struct SamplerParams {
    #[id = "slot"]
    pub slot: IntParam,

    #[id = "pick"]
    pub pick_to_slot: BoolParam,

    #[id = "reload"]
    pub reload_slot: BoolParam,

    #[id = "mode"]
    pub mode: EnumParam<PlaybackMode>,

    #[id = "play"]
    pub play_mode: EnumParam<PlayMode>,

    #[id = "root"]
    pub root_note: IntParam,

    #[id = "slice_base"]
    pub slice_base_note: IntParam,

    #[id = "slices"]
    pub slice_count: IntParam,

    #[id = "start"]
    pub start_pct: FloatParam,

    #[id = "end"]
    pub end_pct: FloatParam,

    #[id = "snap"]
    pub snap_zero: BoolParam,

    #[id = "reverse"]
    pub reverse: BoolParam,

    #[id = "normalize"]
    pub normalize: BoolParam,

    #[id = "loop_mode"]
    pub loop_mode: EnumParam<LoopMode>,

    #[id = "loop_start"]
    pub loop_start_pct: FloatParam,

    #[id = "loop_end"]
    pub loop_end_pct: FloatParam,

    #[id = "att"]
    pub attack_ms: FloatParam,

    #[id = "dec"]
    pub decay_ms: FloatParam,

    #[id = "sus"]
    pub sustain: FloatParam,

    #[id = "rel"]
    pub release_ms: FloatParam,

    #[id = "pitch_amt"]
    pub pitch_env_amt_st: FloatParam,

    #[id = "pitch_dec"]
    pub pitch_env_decay_ms: FloatParam,

    #[id = "filter"]
    pub filter: EnumParam<FilterKind>,

    #[id = "cutoff"]
    pub cutoff_hz: FloatParam,

    #[id = "res"]
    pub resonance: FloatParam,

    #[id = "gain"]
    pub gain: FloatParam,

    #[id = "pan"]
    pub pan: FloatParam,

    #[id = "tune"]
    pub tune_st: FloatParam,

    #[id = "bend"]
    pub bend_range_st: IntParam,

    #[id = "glide"]
    pub glide_ms: FloatParam,

    #[id = "voices"]
    pub max_voices: IntParam,

    #[id = "vel_sens"]
    pub vel_sens: FloatParam,

    #[id = "vel_curve"]
    pub vel_curve: FloatParam,

    #[id = "choke"]
    pub choke_group: IntParam,
}

impl Default for SamplerParams {
    fn default() -> Self {
        Self {
            slot: {
                use std::sync::Arc;
                IntParam::new("Slot", 0, IntRange::Linear { min: 0, max: 127 })
                    .with_value_to_string(Arc::new(|idx| crate::slot_name_for_index(idx)))
            },

            pick_to_slot: BoolParam::new("Pick To Slot", false),
            reload_slot: BoolParam::new("Reload Slot", false),

            mode: EnumParam::new("Mode", PlaybackMode::Classic),
            play_mode: EnumParam::new("Play", PlayMode::Poly),

            root_note: IntParam::new("Root", 60, IntRange::Linear { min: 0, max: 127 }),
            slice_base_note: IntParam::new("Slice Base", 36, IntRange::Linear { min: 0, max: 127 }),
            slice_count: IntParam::new("Slices", 16, IntRange::Linear { min: 1, max: 64 }),

            start_pct: FloatParam::new(
                "Start",
                0.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 100.0,
                },
            )
            .with_unit("%"),

            end_pct: FloatParam::new(
                "End",
                100.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 100.0,
                },
            )
            .with_unit("%"),

            snap_zero: BoolParam::new("Snap Zero", true),
            reverse: BoolParam::new("Reverse", false),
            normalize: BoolParam::new("Normalize", false),

            loop_mode: EnumParam::new("Loop", LoopMode::Off),

            loop_start_pct: FloatParam::new(
                "Loop Start",
                0.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 100.0,
                },
            )
            .with_unit("%"),

            loop_end_pct: FloatParam::new(
                "Loop End",
                100.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 100.0,
                },
            )
            .with_unit("%"),

            attack_ms: FloatParam::new(
                "Attack",
                2.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 5000.0,
                    factor: 0.3,
                },
            )
            .with_unit(" ms"),

            decay_ms: FloatParam::new(
                "Decay",
                100.0,
                FloatRange::Skewed {
                    min: 1.0,
                    max: 8000.0,
                    factor: 0.3,
                },
            )
            .with_unit(" ms"),

            sustain: FloatParam::new("Sustain", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 }),

            release_ms: FloatParam::new(
                "Release",
                200.0,
                FloatRange::Skewed {
                    min: 1.0,
                    max: 12000.0,
                    factor: 0.3,
                },
            )
            .with_unit(" ms"),

            pitch_env_amt_st: FloatParam::new(
                "Pitch Env",
                0.0,
                FloatRange::Linear {
                    min: -48.0,
                    max: 48.0,
                },
            )
            .with_unit(" st"),

            pitch_env_decay_ms: FloatParam::new(
                "Pitch Decay",
                50.0,
                FloatRange::Skewed {
                    min: 1.0,
                    max: 3000.0,
                    factor: 0.3,
                },
            )
            .with_unit(" ms"),

            filter: EnumParam::new("Filter", FilterKind::Off),

            cutoff_hz: FloatParam::new(
                "Cutoff",
                20000.0,
                FloatRange::Skewed {
                    min: 20.0,
                    max: 20000.0,
                    factor: 0.2,
                },
            )
            .with_unit(" Hz"),

            resonance: FloatParam::new(
                "Resonance",
                0.707,
                FloatRange::Linear { min: 0.1, max: 8.0 },
            ),

            gain: FloatParam::new("Gain", 0.8, FloatRange::Linear { min: 0.0, max: 2.5 }),

            pan: FloatParam::new(
                "Pan",
                0.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            ),

            tune_st: FloatParam::new(
                "Tune",
                0.0,
                FloatRange::Linear {
                    min: -24.0,
                    max: 24.0,
                },
            )
            .with_unit(" st"),

            bend_range_st: IntParam::new("Bend Range", 2, IntRange::Linear { min: 0, max: 24 })
                .with_unit(" st"),

            glide_ms: FloatParam::new(
                "Glide",
                0.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 2000.0,
                    factor: 0.4,
                },
            )
            .with_unit(" ms"),

            max_voices: IntParam::new("Voices", 32, IntRange::Linear { min: 1, max: 128 }),

            vel_sens: FloatParam::new("Vel Sens", 0.9, FloatRange::Linear { min: 0.0, max: 1.0 }),

            vel_curve: FloatParam::new(
                "Vel Curve",
                1.0,
                FloatRange::Skewed {
                    min: 0.25,
                    max: 4.0,
                    factor: 0.5,
                },
            ),

            choke_group: IntParam::new("Choke", 0, IntRange::Linear { min: 0, max: 16 }),
        }
    }
}
