use crate::dsp::hermite;

#[derive(Clone)]
pub struct Wave {
    pub data: Vec<f32>,
    #[allow(dead_code)]
    pub channels: usize,
    pub sample_rate: u32,
    pub num_frames: usize,
    pub path: String,
    pub peak_gain: f32,
    pub transients: Vec<usize>,
}

impl Wave {
    pub fn empty() -> Self {
        Self {
            data: Vec::new(),
            channels: 2,
            sample_rate: 44100,
            num_frames: 0,
            path: String::from("<empty>"),
            peak_gain: 1.0,
            transients: Vec::new(),
        }
    }

    pub fn from_stereo(
        data: Vec<f32>,
        sample_rate: u32,
        path: String,
    ) -> Self {
        let num_frames = data.len() / 2;
        let peak = data
            .iter()
            .fold(0.0f32, |acc, s| acc.max(s.abs()));

        let peak_gain = if peak > 1e-8 { 1.0 / peak } else { 1.0 };

        let mut wave = Self {
            data,
            channels: 2,
            sample_rate,
            num_frames,
            path,
            peak_gain,
            transients: Vec::new(),
        };

        wave.transients = analyze_transients(&wave);
        wave
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.num_frames == 0
    }

    #[inline]
    pub fn get_stereo_frame(&self, frame: usize) -> (f32, f32) {
        if self.num_frames == 0 {
            return (0.0, 0.0);
        }

        let f = frame.min(self.num_frames - 1);
        let idx = f * 2;
        (self.data[idx], self.data[idx + 1])
    }

    #[inline]
    pub fn get_mono_frame(&self, frame: usize) -> f32 {
        let (l, r) = self.get_stereo_frame(frame);
        (l + r) * 0.5
    }

    #[inline]
    pub fn sample_stereo(&self, pos: f64) -> (f32, f32) {
        if self.num_frames == 0 {
            return (0.0, 0.0);
        }

        let idx = pos.floor() as isize;
        let frac = (pos - idx as f64) as f32;

        let get = |frame: isize, ch: usize| -> f32 {
            let f = frame.clamp(0, self.num_frames.saturating_sub(1) as isize) as usize;
            self.data[f * 2 + ch]
        };

        let i0 = idx - 1;
        let i1 = idx;
        let i2 = idx + 1;
        let i3 = idx + 2;

        let l = hermite(get(i0, 0), get(i1, 0), get(i2, 0), get(i3, 0), frac);
        let r = hermite(get(i0, 1), get(i1, 1), get(i2, 1), get(i3, 1), frac);

        (l, r)
    }

    pub fn snap_to_zero(&self, frame: usize) -> usize {
        if self.num_frames == 0 {
            return frame;
        }

        let frame = frame.min(self.num_frames - 1);
        let search = (self.sample_rate as usize / 200).clamp(64, 512);

        let start = frame.saturating_sub(search);
        let end = (frame + search).min(self.num_frames - 1);

        let mut best = frame;
        let mut best_amp = self.get_mono_frame(frame).abs();

        for f in start..=end {
            let amp = self.get_mono_frame(f).abs();
            if amp < best_amp {
                best_amp = amp;
                best = f;

                if amp < 0.0005 {
                    break;
                }
            }
        }

        best
    }
}

fn analyze_transients(wave: &Wave) -> Vec<usize> {
    if wave.num_frames < 2048 {
        return Vec::new();
    }

    let hop = 128usize;
    let win = 256usize;
    let min_distance = (wave.sample_rate as usize / 20).max(512);

    let mut energies = Vec::new();
    let mut frame_positions = Vec::new();

    let mut pos = 0usize;
    while pos + win < wave.num_frames {
        let mut e = 0.0f32;

        for i in 0..win {
            let s = wave.get_mono_frame(pos + i);
            e += s * s;
        }

        e = (e / win as f32).sqrt();
        energies.push(e);
        frame_positions.push(pos);

        pos += hop;
    }

    if energies.len() < 4 {
        return Vec::new();
    }

    let avg = energies.iter().copied().sum::<f32>() / energies.len() as f32;
    let threshold = (avg * 1.8).max(0.015);

    let mut transients = Vec::new();
    transients.push(0);

    for i in 2..energies.len() {
        let prev = energies[i - 1];
        let cur = energies[i];

        let flux = cur - prev;

        if cur > threshold && flux > threshold * 0.35 {
            let frame = frame_positions[i];

            if transients
                .last()
                .map(|last| frame.saturating_sub(*last) >= min_distance)
                .unwrap_or(true)
            {
                transients.push(frame);
            }
        }

        if transients.len() >= 128 {
            break;
        }
    }

    transients
}
