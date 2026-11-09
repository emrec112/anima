//! Tempo and beat phase of the desktop audio, for moving to music. A
//! streaming port of phonk-editor's `BeatAnalyzer`, run on the audio capture
//! thread at the sample rate (so it doesn't depend on the render frame rate):
//!
//! 1. every 512 samples, how much louder the sound got: the onset curve.
//!    phonk-editor takes the log energy of the bass and of the whole signal;
//!    here it's the log spectrum in quarter-octave bands, all weighted the
//!    same. Two notes held in one band make its energy beat at their
//!    difference frequency, which reads as a steady fake beat in soft music,
//!    and soft music doesn't hit the bass on every beat;
//! 2. the beat length at which the last 10 s of that curve best matches a
//!    shifted copy of itself, favouring ~135 bpm (so half and double time
//!    don't win by accident) and the tempo we're already on (so a song
//!    doesn't flip between them), refined between frames, and the median of
//!    the last few seconds of those so one odd reading doesn't flip it;
//! 3. the beat grid of that length that lands on the most onset, recent
//!    beats counting most, which a beat clock follows.
//!
//! The render thread reads the result with [`current`].

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Instant;

const HOP: usize = 512;
// FFT length: 21.5 Hz bins, so held notes rarely share one
const SPAN: usize = 2048;
// bins up to ~11 kHz count, in bands about a quarter octave wide from
// 40 Hz: a band of many bins doesn't flicker with noise the way one bin does
const TOP_BIN: usize = SPAN / 4;
const LOW_HZ: f32 = 40.0;
const BANDS: usize = 34;
// log compression: ln(1 + COMPRESS * band / long-term level). Stronger makes
// quiet eighth notes count as much as the beat
const COMPRESS: f32 = 10.0;
// frames from an onset to its flux peak (measured on click tracks)
const LAG: f32 = (SPAN / 2 / HOP) as f32 - 1.0;
const WINDOW_SEC: f32 = 10.0;
// tempos considered, and the one preferred when a half or double fits too
const MIN_BPM: f32 = 68.0;
const MAX_BPM: f32 = 185.0;
const PREFERRED_BPM: f32 = 135.0;
const SPREAD_OCTAVES: f32 = 0.75;
// how much a beat length within ~4 % of the current one is favoured
const KEEP: f32 = 1.0;
// this much silence is a new song
const SONG_GAP_SEC: f32 = 1.5;
// a tempo estimate every half second, from at least 4 s of audio
const ESTIMATE_EVERY_SEC: f32 = 0.5;
const MIN_HISTORY_SEC: f32 = 4.0;
// medians over this many estimates
const ESTIMATES: usize = 9;
// beat grid: re-placed every few frames; each older beat counts this much
// less than the next
const ALIGN_EVERY: usize = 4;
const BEAT_DECAY: f32 = 0.85;

/// What the render thread needs: tempo, where in the beat we are, and how
/// sure the tracker is.
#[derive(Clone, Copy, Debug, Default)]
pub struct Beat {
    pub bpm: f32,
    /// How clearly the audio repeats at that tempo, 0..1 (about 0.3 and up
    /// is a clear beat).
    pub confidence: f32,
    // beats counted at `at`: whole beats plus the phase
    pos: f64,
    at: Option<Instant>,
}

impl Beat {
    /// Beats since the tracker started, at `now`: the whole part counts
    /// beats (a new one has just started when it goes up), the fraction is
    /// the phase, 0 on the beat.
    pub fn position(&self, now: Instant) -> f64 {
        let dt = self.at.map_or(0.0, |at| now.saturating_duration_since(at).as_secs_f64());
        self.pos + dt * f64::from(self.bpm) / 60.0
    }
}

static CURRENT: Mutex<Beat> = Mutex::new(Beat { bpm: 120.0, confidence: 0.0, pos: 0.0, at: None });

/// The latest beat from the audio thread.
pub fn current() -> Beat {
    CURRENT.lock().map(|b| *b).unwrap_or_default()
}

pub fn publish(beat: Beat) {
    if let Ok(mut b) = CURRENT.lock() {
        *b = beat;
    }
}

pub struct Tracker {
    // frames per second
    fps: f32,
    // the last SPAN samples, as a ring
    samples: Vec<f32>,
    next: usize,
    filled: usize,
    hann: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    // first bin of each band, and of the one after the last
    edges: Vec<usize>,
    // long-term mean band magnitude, so loudness doesn't depend on the volume
    level: f32,
    last: Vec<f32>,
    silent_frames: usize,
    heard: usize,
    // onset strength per frame, the last WINDOW_SEC
    rise: VecDeque<f32>,
    window: usize,
    frames: usize,
    estimates: VecDeque<f32>,
    // frames per beat
    period: f32,
    // beats so far plus the phase, at the newest frame
    pos: f64,
    confidence: f32,
    // has settled on a tempo
    locked: bool,
}

impl Tracker {
    pub fn new(sample_rate: f32) -> Self {
        let fps = sample_rate / HOP as f32;
        let window = (WINDOW_SEC * fps) as usize;
        let bin = |hz: f32| (hz * SPAN as f32 / sample_rate).round() as usize;
        let top = TOP_BIN as f32 * sample_rate / SPAN as f32;
        let mut edges: Vec<usize> = (0..=BANDS).map(|b| bin(LOW_HZ * (top / LOW_HZ).powf(b as f32 / BANDS as f32))).collect();
        // at least a bin each
        for b in 1..edges.len() {
            edges[b] = edges[b].max(edges[b - 1] + 1);
        }
        Self {
            fps,
            samples: vec![0.0; SPAN],
            next: 0,
            filled: 0,
            hann: (0..SPAN).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / SPAN as f32).cos()).collect(),
            re: vec![0.0; SPAN],
            im: vec![0.0; SPAN],
            edges,
            level: 0.0,
            last: vec![0.0; BANDS],
            silent_frames: 0,
            heard: 0,
            rise: VecDeque::with_capacity(window + 1),
            window,
            frames: 0,
            estimates: VecDeque::new(),
            period: fps / 2.0,
            pos: 0.0,
            confidence: 0.0,
            locked: false,
        }
    }

    pub fn bpm(&self) -> f32 {
        60.0 * self.fps / self.period
    }

    /// Where things stand as of the newest sample fed, which is taken to be
    /// playing now.
    pub fn beat(&self) -> Beat {
        Beat { bpm: self.bpm(), confidence: self.confidence, pos: self.pos, at: Some(Instant::now()) }
    }

    /// Feed mono samples. While `ignore` (our own voice playing), they keep
    /// time but don't count as onsets.
    pub fn feed(&mut self, samples: &[f32], ignore: bool) {
        for &s in samples {
            self.samples[self.next] = s;
            self.next = (self.next + 1) % SPAN;
            self.filled += 1;
            if self.filled == HOP {
                self.frame(ignore);
            }
        }
    }

    fn frame(&mut self, ignore: bool) {
        self.filled = 0;
        for i in 0..SPAN {
            self.re[i] = self.samples[(self.next + i) % SPAN] * self.hann[i];
            self.im[i] = 0.0;
        }
        crate::fft::fft_in_place(&mut self.re, &mut self.im);
        let band = |b: usize| {
            let bins = self.edges[b]..self.edges[b + 1];
            let n = bins.len() as f32;
            (bins.map(|i| self.re[i] * self.re[i] + self.im[i] * self.im[i]).sum::<f32>() / n).sqrt()
        };
        let bands: [f32; BANDS] = std::array::from_fn(band);
        let mean = bands.iter().sum::<f32>() / BANDS as f32;
        let silent = mean < 1e-4;
        if silent {
            self.silent_frames += 1;
            if self.silent_frames as f32 > SONG_GAP_SEC * self.fps {
                // whatever plays next starts from scratch
                self.locked = false;
                self.estimates.clear();
            }
        } else {
            self.silent_frames = 0;
            // the average of the last ~10 s of sound (of all of it, before
            // there's that much: a fade-in mustn't set it low for long)
            self.heard += 1;
            let k = 1.0 / (self.heard as f32).min(self.fps * 10.0);
            self.level += (mean - self.level) * k;
        }
        // log-compressed like phonk-editor's ln(1 + 1000 E), but relative to
        // the song's own level so the volume doesn't matter
        let scale = COMPRESS / self.level.max(1e-4);
        let mut flux = 0.0;
        for (b, m) in bands.iter().enumerate() {
            let now = (1.0 + scale * m).ln();
            flux += (now - self.last[b]).max(0.0);
            self.last[b] = now;
        }
        let rise = if silent || ignore { 0.0 } else { flux / BANDS as f32 };
        self.rise.push_back(rise);
        if self.rise.len() > self.window {
            self.rise.pop_front();
        }

        self.frames += 1;
        self.pos += 1.0 / f64::from(self.period);
        if self.frames % ALIGN_EVERY == 0 {
            self.align();
        }
        let every = (ESTIMATE_EVERY_SEC * self.fps) as usize;
        if self.frames % every == 0 && self.rise.len() as f32 >= MIN_HISTORY_SEC * self.fps {
            self.estimate();
        }
    }

    /// Beat length from the onset curve's autocorrelation (phonk-editor's
    /// `estimateTempo`).
    fn estimate(&mut self) {
        let n = self.rise.len();
        let mean = self.rise.iter().sum::<f32>() / n as f32;
        let spread = (self.rise.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / n as f32).sqrt();
        // one huge onset (the music starting after silence) would line up
        // with whatever comes after it: cap them
        let cap = mean + 8.0 * spread;
        let r: Vec<f32> = self.rise.iter().map(|v| v.min(cap)).collect();
        let mean = r.iter().sum::<f32>() / n as f32;
        // smoothed over ±2 frames: a beat length between two whole frames
        // would otherwise split its peak in two and lose to its double,
        // which can land on a whole frame
        let e: Vec<f32> = (0..n)
            .map(|i| {
                let at = |d: isize| r[(i as isize + d).clamp(0, n as isize - 1) as usize];
                (at(-2) + 4.0 * at(-1) + 6.0 * at(0) + 4.0 * at(1) + at(2)) / 16.0 - mean
            })
            .collect();
        let ac = |lag: usize| -> f32 { (lag..n).map(|i| e[i] * e[i - lag]).sum::<f32>() / (n - lag) as f32 };
        let zero = ac(0);
        if zero <= 1e-9 {
            // silence: keep the tempo, lose the confidence
            self.confidence *= 0.8;
            return;
        }
        let shortest = (self.fps * 60.0 / MAX_BPM).floor() as usize;
        let longest = (self.fps * 60.0 / MIN_BPM).ceil() as usize;
        let sim: Vec<f32> = (0..=longest + 1).map(|lag| if lag + 1 >= shortest { ac(lag) } else { 0.0 }).collect();
        let plausible = |bpm: f32| (-0.5 * ((bpm / PREFERRED_BPM).log2() / SPREAD_OCTAVES).powi(2)).exp();
        let mut best = (f32::NEG_INFINITY, shortest);
        // real peaks only: the edge of the range isn't one
        for lag in (shortest..=longest).filter(|&l| sim[l] >= sim[l - 1] && sim[l] >= sim[l + 1]) {
            // the tempo we're on counts double, so a song doesn't flip
            // between its half, double or 3:2 time from one estimate to
            // the next
            let near = (lag as f32 / self.period).ln() / 0.04;
            let keep = 1.0 + KEEP * (-0.5 * near * near).exp() * f32::from(self.locked);
            let score = sim[lag] * plausible(60.0 * self.fps / lag as f32) * keep;
            if score > best.0 {
                best = (score, lag);
            }
        }
        if best.0 == f32::NEG_INFINITY {
            return;
        }
        let lag = best.1;
        let (before, peak, after) = (sim[lag - 1], sim[lag], sim[lag + 1]);
        let curve = before - 2.0 * peak + after;
        let offset = if curve.abs() > 1e-12 { 0.5 * (before - after) / curve } else { 0.0 };
        let len = lag as f32 + if offset.abs() < 1.0 { offset } else { 0.0 };
        self.confidence += ((peak / zero).clamp(0.0, 1.0) - self.confidence) * 0.3;

        self.estimates.push_back(len);
        if self.estimates.len() > ESTIMATES {
            self.estimates.pop_front();
        }
        let mut sorted: Vec<f32> = self.estimates.iter().copied().collect();
        sorted.sort_by(f32::total_cmp);
        let median = sorted[sorted.len() / 2];
        // only once there's a whole window to go on: the first few
        // seconds can easily read double or half
        self.locked = self.estimates.len() == ESTIMATES && self.rise.len() >= self.window;
        let old = self.bpm();
        // settle quickly on a new song, smoothly otherwise
        let k = if (median / self.period - 1.0).abs() > 0.08 { 0.7 } else { 0.3 };
        self.period += (median - self.period) * k;
        if (self.bpm() - old).abs() > 3.0 {
            log::debug!("tempo: {:.0} bpm ({:.2} sure)", self.bpm(), self.confidence);
        }
    }

    /// Place the beat grid where it lands on the most rise (phonk-editor's
    /// `placeBeatGrid`, recent beats weighted most) and pull the clock
    /// toward it.
    fn align(&mut self) {
        let n = self.rise.len();
        let p = self.period;
        if (n as f32) < p * 3.0 {
            return;
        }
        // lightly smoothed, so a beat a frame off still counts
        let at = |i: usize| {
            let r = &self.rise;
            0.5 * r[i] + 0.25 * (r[i.saturating_sub(1)] + r[(i + 1).min(n - 1)])
        };
        let newest = (n - 1) as f32;
        let mut best = (0.0f32, 0.0f32);
        let mut total = 0.0;
        let mut o = 0.0;
        while o < p {
            let (mut sum, mut w, mut x) = (0.0, 1.0, newest - o);
            while x >= 0.0 {
                sum += w * at(x.round() as usize);
                w *= BEAT_DECAY;
                x -= p;
            }
            total += sum;
            if sum > best.0 {
                best = (sum, o);
            }
            o += 0.5;
        }
        // how much the best grid stands out from the average one
        let average = total / (p * 2.0).ceil();
        if best.0 <= average * 1.2 {
            return;
        }
        // the last beat was `best.1` frames before the newest window's flux
        // peak, which an onset reaches a hop before the window's centre
        let phase = (best.1 + LAG) / p;
        let mut err = phase - self.pos.rem_euclid(1.0) as f32;
        err -= err.round();
        self.pos += f64::from(err * 0.25);
    }
}

#[cfg(test)]
mod tests {
    //! Offline checks on synthetic music: `cargo test --release --lib tempo -- --nocapture`.
    //! Real files (decoded with ffmpeg):
    //! `TEMPO_FILE=song.mp3 cargo test --release --lib tempo::tests::file -- --ignored --nocapture`.
    use super::*;
    use std::f32::consts::PI;

    const RATE: f32 = 44_100.0;

    struct Rng(u32);
    impl Rng {
        fn next(&mut self) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 17;
            self.0 ^= self.0 << 5;
            self.0 as f32 / u32::MAX as f32 * 2.0 - 1.0
        }
    }

    fn add(out: &mut [f32], at: f32, sound: impl Fn(f32) -> f32, len: f32) {
        let start = (at * RATE) as usize;
        let n = (len * RATE) as usize;
        for i in 0..n {
            if let Some(o) = out.get_mut(start + i) {
                // faded out over the last fifth: a hard cut is a click
                let fade = ((n - i) as f32 / (n as f32 * 0.2)).min(1.0);
                *o += sound(i as f32 / RATE) * fade;
            }
        }
    }

    fn kick(t: f32) -> f32 {
        0.9 * (2.0 * PI * (50.0 * t + 120.0 / 30.0 * (1.0 - (-t * 30.0).exp()))).sin() * (-t / 0.12).exp()
    }

    fn noise_burst(seed: u32, gain: f32, decay: f32) -> impl Fn(f32) -> f32 {
        let mut rng = Rng(seed);
        let n: Vec<f32> = (0..(0.3 * RATE) as usize).map(|_| rng.next()).collect();
        move |t| n[((t * RATE) as usize).min(n.len() - 1)] * gain * (-t / decay).exp()
    }

    // a soft piano-ish note: harmonics, a 15 ms attack, a long decay
    fn note(freq: f32, gain: f32) -> impl Fn(f32) -> f32 {
        move |t| {
            let env = (t / 0.015).min(1.0) * (-t / 0.6).exp();
            (1..=5).map(|h| (2.0 * PI * freq * h as f32 * t).sin() / (h * h) as f32).sum::<f32>() * gain * env
        }
    }

    /// Beat times from `bpm(t)` over `secs`.
    fn beat_times(secs: f32, bpm: impl Fn(f32) -> f32) -> Vec<f32> {
        let mut t = 0.3;
        let mut out = Vec::new();
        while t < secs {
            out.push(t);
            t += 60.0 / bpm(t);
        }
        out
    }

    fn drums(secs: f32, beats: &[f32], swing: bool, synco: bool) -> Vec<f32> {
        let mut out = vec![0.0; (secs * RATE) as usize];
        for (i, w) in beats.windows(2).enumerate() {
            let (b, len) = (w[0], w[1] - w[0]);
            if !synco || i % 4 != 2 {
                add(&mut out, b, kick, 0.5);
            }
            if synco && i % 4 == 1 {
                // the kick on the "and" of 2 instead of on 3
                add(&mut out, b + len * 0.5, kick, 0.5);
            }
            if i % 2 == 1 {
                add(&mut out, b, noise_burst(i as u32 + 7, 0.35, 0.08), 0.3);
            }
            add(&mut out, b, noise_burst(i as u32 + 99, 0.08, 0.01), 0.05);
            let off = if swing { 2.0 / 3.0 } else { 0.5 };
            add(&mut out, b + len * off, noise_burst(i as u32 + 199, 0.06, 0.01), 0.05);
        }
        out
    }

    // drumless: a soft chord each bar, a melody note on each beat and a
    // quieter arpeggio on the eighths, a pad underneath and some hiss
    fn soft(secs: f32, beats: &[f32]) -> Vec<f32> {
        let mut out = vec![0.0; (secs * RATE) as usize];
        let chords = [[220.0, 277.2, 329.6], [196.0, 246.9, 293.7], [174.6, 220.0, 261.6], [196.0, 246.9, 311.1]];
        let melody = [2.0, 1.5, 1.68, 1.33, 2.0, 2.24, 1.5, 1.26];
        for (i, w) in beats.windows(2).enumerate() {
            let (b, len) = (w[0], w[1] - w[0]);
            let chord = chords[(i / 4) % 4];
            if i % 4 == 0 {
                for f in chord {
                    add(&mut out, b, note(f / 2.0, 0.05), 2.0);
                }
            }
            add(&mut out, b, note(chord[0] * melody[i % 8], 0.06), 1.0);
            add(&mut out, b + len * 0.5, note(chord[(i + 1) % 3] * 2.0, 0.025), 1.0);
        }
        let mut rng = Rng(5);
        for (i, o) in out.iter_mut().enumerate() {
            let t = i as f32 / RATE;
            *o += 0.03 * ((2.0 * PI * 110.0 * t).sin() + (2.0 * PI * 164.8 * t).sin()) + 0.004 * rng.next();
        }
        out
    }

    struct Run {
        bpms: Vec<(f32, f32)>,
        // predicted beat starts, seconds
        beats: Vec<f32>,
        confidence: f32,
        // onset strength by beat phase (eighths of a beat, 0 on the beat)
        by_phase: [f32; 8],
    }

    /// Feed like the capture thread does (1024-sample reads) and note the
    /// tempo every half second and when each beat starts.
    fn run(samples: &[f32]) -> Run {
        let mut t = Tracker::new(RATE);
        let mut r = Run { bpms: Vec::new(), beats: Vec::new(), confidence: 0.0, by_phase: [0.0; 8] };
        let mut last = 0i64;
        for (i, chunk) in samples.chunks(1024).enumerate() {
            t.feed(chunk, false);
            let now = (i * 1024 + chunk.len()) as f32 / RATE;
            if t.pos.floor() as i64 > last {
                last = t.pos.floor() as i64;
                // when the beat started, within this chunk
                let back = t.pos.rem_euclid(1.0) as f32 * t.period / t.fps;
                r.beats.push(now - back);
            }
            if i % 22 == 0 {
                r.bpms.push((now, t.bpm()));
            }
            // the two frames this chunk made, at the beat position of the
            // onsets they show
            for k in 0..2.min(t.rise.len()) {
                let rise = t.rise[t.rise.len() - 1 - k];
                let pos = t.pos - f64::from((k as f32 + LAG) / t.period);
                r.by_phase[((pos.rem_euclid(1.0) * 8.0 + 0.5) as usize) % 8] += rise;
            }
        }
        r.confidence = t.confidence;
        r
    }

    /// Median and range of the tempo after `from` s, how far the predicted
    /// beats are from the real ones (mean, ms) and the share within 70 ms.
    fn score(run: &Run, truth: &[f32], from: f32) -> (f32, f32, f32, f32, f32) {
        let mut after: Vec<f32> = run.bpms.iter().filter(|b| b.0 >= from).map(|b| b.1).collect();
        after.sort_by(f32::total_cmp);
        let (lo, hi, med) = (after[0], after[after.len() - 1], after[after.len() / 2]);
        let errs: Vec<f32> = truth
            .iter()
            .filter(|&&b| b >= from)
            .map(|&b| run.beats.iter().map(|&p| (p - b).abs()).fold(f32::MAX, f32::min))
            .collect();
        let mean = errs.iter().sum::<f32>() / errs.len() as f32 * 1000.0;
        let hit = errs.iter().filter(|&&e| e < 0.07).count() as f32 / errs.len() as f32;
        (med, lo, hi, mean, hit)
    }

    fn report(name: &str, truth_bpm: f32, run: &Run, truth: &[f32], from: f32, failures: &mut Vec<String>) {
        report_or_double(name, truth_bpm, run, truth, from, failures, false);
    }

    fn report_or_double(name: &str, truth_bpm: f32, run: &Run, truth: &[f32], from: f32, failures: &mut Vec<String>, double_ok: bool) {
        let (med, lo, hi, err, hit) = score(run, truth, from);
        println!(
            "| {name:<28} | {truth_bpm:>5} | {med:>6.1} | {lo:>6.1}-{hi:<6.1} | {err:>4.0} ms | {:>4.0} % | {:.2} |",
            hit * 100.0,
            run.confidence
        );
        let off = (med - truth_bpm).abs() > 1.5 && !(double_ok && (med - 2.0 * truth_bpm).abs() < 3.0);
        if off || err > 40.0 {
            failures.push(name.to_string());
        }
    }

    #[test]
    fn synthetic() {
        println!("| signal | truth | median bpm | range | beat error | beats within 70 ms | confidence |");
        let secs = 40.0;
        let mut failures = Vec::new();
        for bpm in [72.0, 90.0, 100.0, 120.0, 128.0, 140.0, 150.0, 163.0, 175.0] {
            let truth = beat_times(secs, |_| bpm);
            report(&format!("drums {bpm}"), bpm, &run(&drums(secs, &truth, false, false)), &truth, 12.0, &mut failures);
        }
        for bpm in [96.0, 128.0, 163.0] {
            let truth = beat_times(secs, |_| bpm);
            report(&format!("swung + syncopated {bpm}"), bpm, &run(&drums(secs, &truth, true, true)), &truth, 12.0, &mut failures);
        }
        for bpm in [80.0, 120.0, 145.0, 163.0] {
            let truth = beat_times(secs, |_| bpm);
            // known: slow and soft with an even eighth-note arpeggio reads
            // as the eighths (160), still on the beats
            report_or_double(&format!("soft drumless {bpm}"), bpm, &run(&soft(secs, &truth)), &truth, 12.0, &mut failures, bpm < 100.0);
        }
        // 30 dB down: the level doesn't matter
        let truth = beat_times(secs, |_| 140.0);
        let quiet: Vec<f32> = drums(secs, &truth, false, false).iter().map(|s| s * 0.03).collect();
        report("drums 140, -30 dB", 140.0, &run(&quiet), &truth, 12.0, &mut failures);

        // tempo changes
        let secs = 60.0;
        let settled = |r: &Run, at: f32, bpm: f32| {
            r.bpms
                .iter()
                .find(|b| b.0 > at && r.bpms.iter().filter(|c| c.0 >= b.0).all(|c| (c.1 - bpm).abs() < 2.0))
                .map_or(f32::NAN, |b| b.0 - at)
        };
        let truth = beat_times(secs, |t| if t < 30.0 { 120.0 } else { 150.0 });
        let r = run(&drums(secs, &truth, false, false));
        report("120 -> 150 at 30 s, no gap", 150.0, &r, &truth, 45.0, &mut failures);
        println!("  on 150 for good {:.1} s after the change", settled(&r, 30.0, 150.0));
        // a second song after 3 s of silence
        let a = drums(27.0, &beat_times(27.0, |_| 120.0), false, false);
        let b_truth = beat_times(30.0, |_| 150.0);
        let b = soft(30.0, &b_truth);
        let mut gap = a;
        gap.extend(std::iter::repeat_n(0.0, (3.0 * RATE) as usize));
        gap.extend(&b);
        let truth: Vec<f32> = b_truth.iter().map(|t| t + 30.0).collect();
        let r = run(&gap);
        report("drums 120, gap, soft 150", 150.0, &r, &truth, 45.0, &mut failures);
        println!("  on 150 for good {:.1} s after the second song starts", settled(&r, 30.0, 150.0));
        let ramp = |t: f32| 120.0 + 30.0 * (t / 60.0);
        let truth = beat_times(secs, ramp);
        let r = run(&drums(secs, &truth, false, false));
        let (_, _, _, err, hit) = score(&r, &truth, 15.0);
        println!("| ramp 120 -> 150 over 60 s | ramp | beat error {err:.0} ms, {:.0} % within 70 ms |", hit * 100.0);
        for (t, b) in r.bpms.iter().filter(|b| [20, 30, 40, 50, 59].contains(&(b.0 as i32)) && b.0.fract() < 0.03) {
            println!("  t={t:>4.0} s: {b:.1} bpm (truth {:.1})", ramp(*t));
        }
        if err > 40.0 {
            failures.push("ramp".into());
        }
        assert!(failures.is_empty(), "off: {failures:?}");
    }

    #[test]
    #[ignore]
    fn file() {
        let path = std::env::var("TEMPO_FILE").expect("TEMPO_FILE");
        let out = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-t", "240", "-i", &path, "-ac", "1", "-ar", "44100", "-f", "f32le", "-"])
            .output()
            .expect("ffmpeg");
        let samples: Vec<f32> = out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        let r = run(&samples);
        let mut after: Vec<f32> = r.bpms.iter().filter(|b| b.0 >= 15.0).map(|b| b.1).collect();
        after.sort_by(f32::total_cmp);
        let med = after[after.len() / 2];
        let within = after.iter().filter(|b| (*b - med).abs() <= 2.0).count() as f32 / after.len() as f32;
        // how much of the onset strength falls on the predicted beat, against
        // an even spread (1.0): a clock that drifts off the beat smears it out
        let total: f32 = r.by_phase.iter().sum();
        println!(
            "RESULT median {med:.1} p10-p90 {:.1}-{:.1} within2 {:.0}% on-beat {:.2}x conf {:.2}",
            after[after.len() / 10],
            after[after.len() * 9 / 10],
            within * 100.0,
            r.by_phase[0] / total * 8.0,
            r.confidence
        );
    }
}
