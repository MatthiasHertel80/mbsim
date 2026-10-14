//! Speaker output: the MLT-8530 is driven through a MOSFET from P0.00. Samples are the exact
//! fraction of each sample period in which the gate is high (box-filtered edges), followed by a
//! DC blocker, and are played through the host's default audio output.

use mbsim_core::time::TICKS_PER_SEC;
use mbsim_core::Ticks;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

pub struct Synth {
    rate: f64,
    /// Start of the sample being accumulated, and how far it is integrated (ticks).
    t: f64,
    pos: f64,
    level: bool,
    acc: f64,
    hp_prev_in: f32,
    hp_prev_out: f32,
    pub volume: f32,
    pub muted: bool,
    buf: Arc<Mutex<VecDeque<f32>>>,
    #[cfg(feature = "audio")]
    _stream: Option<cpal::Stream>,
    pub available: bool,
    /// Recent output level (for the GUI).
    pub activity: f32,
}

impl Synth {
    pub fn new() -> Synth {
        let buf = Arc::new(Mutex::new(VecDeque::new()));
        #[cfg(feature = "audio")]
        let (stream, rate) = match open_output(buf.clone()) {
            Ok((s, r)) => (Some(s), r),
            Err(e) => {
                log::warn!("audio output not available: {e}");
                (None, 48_000.0)
            }
        };
        #[cfg(not(feature = "audio"))]
        let rate = 48_000.0;
        Synth {
            rate,
            t: 0.0,
            pos: 0.0,
            level: false,
            acc: 0.0,
            hp_prev_in: 0.0,
            hp_prev_out: 0.0,
            volume: 0.4,
            muted: false,
            buf,
            #[cfg(feature = "audio")]
            available: stream.is_some(),
            #[cfg(not(feature = "audio"))]
            available: false,
            #[cfg(feature = "audio")]
            _stream: stream,
            activity: 0.0,
        }
    }

    fn period(&self) -> f64 {
        TICKS_PER_SEC as f64 / self.rate
    }

    /// Restarts the timeline at `t` (after a pause or a speed change).
    pub fn resync(&mut self, t: Ticks) {
        self.t = t as f64;
        self.pos = self.t;
        self.acc = 0.0;
        if let Ok(mut b) = self.buf.lock() {
            b.clear();
        }
    }

    fn emit(&mut self, v: f32) {
        // DC blocker (fc about 30 Hz)
        let y = v - self.hp_prev_in + 0.996 * self.hp_prev_out;
        self.hp_prev_in = v;
        self.hp_prev_out = y;
        self.activity = self.activity * 0.999 + y.abs() * 0.001;
        let out = if self.muted { 0.0 } else { y * self.volume };
        if let Ok(mut b) = self.buf.lock() {
            b.push_back(out);
            let max = (self.rate * 0.25) as usize;
            if b.len() > max {
                let n = b.len() - max / 2;
                b.drain(..n);
            }
        }
    }

    /// Integrates the gate level up to time `until` (ticks).
    fn run_to(&mut self, until: f64) {
        if until <= self.pos {
            return;
        }
        if until - self.pos > TICKS_PER_SEC as f64 {
            self.t = until;
            self.pos = until;
            self.acc = 0.0;
            return;
        }
        let p = self.period();
        loop {
            let end = self.t + p;
            if until >= end {
                if self.level {
                    self.acc += end - self.pos;
                }
                let v = (self.acc / p) as f32;
                self.emit(v);
                self.t = end;
                self.pos = end;
                self.acc = 0.0;
            } else {
                if self.level {
                    self.acc += until - self.pos;
                }
                self.pos = until;
                break;
            }
        }
    }

    pub fn edge(&mut self, t: Ticks, high: bool) {
        self.run_to(t as f64);
        self.level = high;
    }

    pub fn advance(&mut self, t: Ticks) {
        self.run_to(t as f64);
    }
}

impl Default for Synth {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "audio")]
fn open_output(buf: Arc<Mutex<VecDeque<f32>>>) -> Result<(cpal::Stream, f64), String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let host = cpal::default_host();
    let dev = host.default_output_device().ok_or("no output device")?;
    let sup = dev.default_output_config().map_err(|e| e.to_string())?;
    let channels = sup.channels() as usize;
    let rate = sup.sample_rate() as f64;
    let fmt = sup.sample_format();
    let config: cpal::StreamConfig = sup.into();
    let err = |e| log::warn!("audio stream error: {e}");
    let stream = match fmt {
        cpal::SampleFormat::F32 => dev.build_output_stream(config, move |data: &mut [f32], _| fill(data, channels, &buf, |v| v), err, None),
        cpal::SampleFormat::I16 => dev.build_output_stream(
            config,
            move |data: &mut [i16], _| fill(data, channels, &buf, |v| (v.clamp(-1.0, 1.0) * 32767.0) as i16),
            err,
            None,
        ),
        cpal::SampleFormat::U16 => dev.build_output_stream(
            config,
            move |data: &mut [u16], _| fill(data, channels, &buf, |v| ((v.clamp(-1.0, 1.0) + 1.0) * 32767.5) as u16),
            err,
            None,
        ),
        f => return Err(format!("unsupported sample format {f:?}")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, rate))
}

#[cfg(feature = "audio")]
fn fill<T: Copy>(data: &mut [T], channels: usize, buf: &Arc<Mutex<VecDeque<f32>>>, conv: impl Fn(f32) -> T) {
    let mut b = match buf.lock() {
        Ok(b) => b,
        Err(_) => return,
    };
    for frame in data.chunks_mut(channels.max(1)) {
        let v = b.pop_front().unwrap_or(0.0);
        for s in frame.iter_mut() {
            *s = conv(v);
        }
    }
}
