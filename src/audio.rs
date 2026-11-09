//! Zero-overhead real-time desktop audio spectrum analyzer & beat detector.
//!
//! Connects to PipeWire / PulseAudio's default output monitor source using
//! `libpulse-simple` loaded dynamically via `dlopen` (so the binary runs even
//! on systems without PulseAudio installed).
//!
//! Optimization guarantee:
//! * When `set_enabled(false)` is set (e.g., static wallpaper, paused, or user
//!   disabled audio capture), the capture thread blocks on a `Condvar` and
//!   releases the PulseAudio stream, using **0% CPU**.
//! * FFT and band integration use pre-allocated fixed-size stack buffers with
//!   zero heap allocations in the hot loop.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::ptr;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

const SAMPLE_RATE: u32 = 44_100;
const FFT_SIZE: usize = 1024;

#[derive(Debug, Clone)]
pub struct AudioSnapshot {
    /// `[bass, low_mid, high_mid, level]` normalized to `0.0..=1.0`.
    pub bands: [f32; 4],
    /// 64 logarithmic frequency bins (`0.0..=1.0`) for Wallpaper Engine
    /// `wallpaperRegisterAudioListener` and bar visualizers.
    pub spectrum: [f32; 64],
    /// Beat / transient impulse strength (`0.0..=1.0`).
    pub beat: f32,
    /// Real-time acoustic vocal viseme parameters:
    /// `[mouth_open (0..1), mouth_form (-1..1: -1=O/U, 0=A, +1=I/E), sibilance (0..1), vocal_activity (0..1)]`.
    pub mouth: [f32; 4],
    /// True when desktop audio energy is above the silence threshold.
    pub desktop_playing: bool,
}

impl Default for AudioSnapshot {
    fn default() -> Self {
        Self {
            bands: [0.0; 4],
            spectrum: [0.0; 64],
            beat: 0.0,
            mouth: [0.0; 4],
            desktop_playing: false,
        }
    }
}

struct SharedState {
    enabled: bool,
    quit: bool,
    snapshot: AudioSnapshot,
}

#[derive(Clone)]
pub struct AudioAnalyzer {
    inner: Arc<(Mutex<SharedState>, Condvar)>,
}

impl AudioAnalyzer {
    pub fn spawn(initially_enabled: bool) -> Self {
        let inner = Arc::new((
            Mutex::new(SharedState {
                enabled: initially_enabled,
                quit: false,
                snapshot: AudioSnapshot::default(),
            }),
            Condvar::new(),
        ));
        let thread_inner = Arc::clone(&inner);
        std::thread::Builder::new()
            .name("anima-audio".into())
            .spawn(move || audio_worker(thread_inner))
            .ok();
        Self { inner }
    }

    pub fn set_enabled(&self, enabled: bool) {
        let (lock, cvar) = &*self.inner;
        if let Ok(mut guard) = lock.lock() {
            if guard.enabled != enabled {
                guard.enabled = enabled;
                if !enabled {
                    guard.snapshot = AudioSnapshot::default();
                }
                cvar.notify_all();
            }
        }
    }

    pub fn snapshot(&self) -> AudioSnapshot {
        let (lock, _) = &*self.inner;
        lock.lock()
            .map(|g| g.snapshot.clone())
            .unwrap_or_default()
    }

    pub fn stop(&self) {
        let (lock, cvar) = &*self.inner;
        if let Ok(mut guard) = lock.lock() {
            guard.quit = true;
            guard.enabled = false;
            cvar.notify_all();
        }
    }
}

#[repr(C)]
struct PaSampleSpec {
    format: c_int,
    rate: u32,
    channels: u8,
}

#[repr(C)]
struct PaBufferAttr {
    maxlength: u32,
    tlength: u32,
    prebuf: u32,
    minreq: u32,
    fragsize: u32,
}

const PA_STREAM_RECORD: c_int = 2;
const PA_SAMPLE_FLOAT32LE: c_int = 5;

type PaSimpleNewFn = unsafe extern "C" fn(
    server: *const c_char,
    name: *const c_char,
    dir: c_int,
    dev: *const c_char,
    stream_name: *const c_char,
    ss: *const PaSampleSpec,
    map: *const c_void,
    attr: *const PaBufferAttr,
    error: *mut c_int,
) -> *mut c_void;

type PaSimpleReadFn =
    unsafe extern "C" fn(s: *mut c_void, data: *mut c_void, bytes: usize, error: *mut c_int) -> c_int;
type PaSimpleFreeFn = unsafe extern "C" fn(s: *mut c_void);

struct PulseLib {
    _handle: *mut c_void,
    simple_new: PaSimpleNewFn,
    simple_read: PaSimpleReadFn,
    simple_free: PaSimpleFreeFn,
}

unsafe impl Send for PulseLib {}

impl PulseLib {
    fn load() -> Option<Self> {
        unsafe {
            let names = [c"libpulse-simple.so.0", c"libpulse-simple.so"];
            let mut handle = ptr::null_mut();
            for name in names {
                handle = libc::dlopen(name.as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
                if !handle.is_null() {
                    break;
                }
            }
            if handle.is_null() {
                return None;
            }
            let sym_new = libc::dlsym(handle, c"pa_simple_new".as_ptr());
            let sym_read = libc::dlsym(handle, c"pa_simple_read".as_ptr());
            let sym_free = libc::dlsym(handle, c"pa_simple_free".as_ptr());
            if sym_new.is_null() || sym_read.is_null() || sym_free.is_null() {
                libc::dlclose(handle);
                return None;
            }
            Some(Self {
                _handle: handle,
                simple_new: std::mem::transmute(sym_new),
                simple_read: std::mem::transmute(sym_read),
                simple_free: std::mem::transmute(sym_free),
            })
        }
    }

    fn open_monitor(&self) -> Option<*mut c_void> {
        let ss = PaSampleSpec {
            format: PA_SAMPLE_FLOAT32LE,
            rate: SAMPLE_RATE,
            channels: 1,
        };
        let frag_bytes = (FFT_SIZE * std::mem::size_of::<f32>()) as u32;
        let attr = PaBufferAttr {
            maxlength: frag_bytes * 4,
            tlength: u32::MAX,
            prebuf: u32::MAX,
            minreq: u32::MAX,
            fragsize: frag_bytes,
        };
        let app_name = c"anima";
        let stream_name = c"wallpaper-audio-spectrum";

        // Try `@DEFAULT_MONITOR@` first (PipeWire & modern PulseAudio), then default source.
        let candidates: [Option<CString>; 2] = [
            CString::new("@DEFAULT_MONITOR@").ok(),
            detect_default_monitor_source(),
        ];

        for dev in candidates.into_iter().flatten() {
            let mut err: c_int = 0;
            let s = unsafe {
                (self.simple_new)(
                    ptr::null(),
                    app_name.as_ptr(),
                    PA_STREAM_RECORD,
                    dev.as_ptr(),
                    stream_name.as_ptr(),
                    &ss,
                    ptr::null(),
                    &attr,
                    &mut err,
                )
            };
            if !s.is_null() {
                return Some(s);
            }
        }
        None
    }
}

fn detect_default_monitor_source() -> Option<CString> {
    let out = std::process::Command::new("pactl")
        .args(["get-default-sink"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let sink = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if sink.is_empty() {
        return None;
    }
    CString::new(format!("{sink}.monitor")).ok()
}

fn audio_worker(inner: Arc<(Mutex<SharedState>, Condvar)>) {
    let Some(pulse) = PulseLib::load() else {
        log::debug!("libpulse-simple not found; audio spectrum disabled");
        return;
    };

    // Precompute Hann window.
    let mut window = [0.0f32; FFT_SIZE];
    for (i, w) in window.iter_mut().enumerate() {
        let phase = 2.0 * std::f32::consts::PI * (i as f32) / (FFT_SIZE as f32);
        *w = 0.5 * (1.0 - phase.cos());
    }

    let mut pcm = [0.0f32; FFT_SIZE];
    let mut re = [0.0f32; FFT_SIZE];
    let mut im = [0.0f32; FFT_SIZE];
    let mut smooth_bands = [0.0f32; 4];
    let mut smooth_spec = [0.0f32; 64];
    let mut energy_ema = 0.01f32;
    let mut smooth_beat = 0.0f32;
    let mut vocal_fast_env = 0.0f32;
    let mut vocal_floor_env = 0.0f32;
    let mut prev_vocal_raw = 0.0f32;
    let mut smooth_mouth = [0.0f32; 4];
    // tempo and beat phase from the raw samples, at the sample rate
    let mut tempo = crate::tempo::Tracker::new(SAMPLE_RATE as f32);

    let (lock, cvar) = &*inner;
    let mut stream: Option<*mut c_void> = None;

    loop {
        // Wait while disabled.
        {
            let mut guard = match lock.lock() {
                Ok(g) => g,
                Err(_) => break,
            };
            while !guard.enabled && !guard.quit {
                if let Some(s) = stream.take() {
                    unsafe { (pulse.simple_free)(s) };
                }
                guard = match cvar.wait(guard) {
                    Ok(g) => g,
                    Err(_) => return,
                };
            }
            if guard.quit {
                if let Some(s) = stream.take() {
                    unsafe { (pulse.simple_free)(s) };
                }
                return;
            }
        }

        if stream.is_none() {
            stream = pulse.open_monitor();
            if stream.is_none() {
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
            // a gap in the audio: start the tempo over
            tempo = crate::tempo::Tracker::new(SAMPLE_RATE as f32);
        }

        let s = stream.unwrap();
        let mut err: c_int = 0;
        let rc = unsafe {
            (pulse.simple_read)(
                s,
                pcm.as_mut_ptr() as *mut c_void,
                FFT_SIZE * std::mem::size_of::<f32>(),
                &mut err,
            )
        };
        if rc < 0 {
            unsafe { (pulse.simple_free)(s) };
            stream = None;
            std::thread::sleep(Duration::from_millis(500));
            continue;
        }

        // our own voice lines are on this audio too: they're not the music's beat
        tempo.feed(&pcm, crate::voice::sounding());
        crate::tempo::publish(tempo.beat());

        // Apply Hann window, compute RMS and Zero-Crossing Rate (ZCR for sibilants).
        let mut rms = 0.0f32;
        let mut zero_crossings = 0u32;
        for i in 0..FFT_SIZE {
            let sample = pcm[i];
            if i > 0 && ((pcm[i - 1] >= 0.0) != (sample >= 0.0)) {
                zero_crossings += 1;
            }
            rms += sample * sample;
            re[i] = sample * window[i];
            im[i] = 0.0;
        }
        rms = (rms / FFT_SIZE as f32).sqrt();
        let zcr = (zero_crossings as f32) / (FFT_SIZE as f32);

        fft_in_place(&mut re, &mut im);

        let half = FFT_SIZE / 2;
        let mut mags = [0.0f32; FFT_SIZE / 2];
        for i in 1..half {
            mags[i] = (re[i] * re[i] + im[i] * im[i]).sqrt() / (half as f32);
        }

        // Frequency resolution: 44100 / 1024 ≈ 43.066 Hz per bin.
        // Bass: ~20..250 Hz (bins 1..6)
        // Low-Mid: ~250..1200 Hz (bins 6..28)
        // High-Mid/Treble: ~1200..8000 Hz (bins 28..186)
        let bass = (avg_slice(&mags[1..6]) * 18.0).clamp(0.0, 1.0);
        let low_mid = (avg_slice(&mags[6..28]) * 28.0).clamp(0.0, 1.0);
        let high_mid = (avg_slice(&mags[28..186]) * 45.0).clamp(0.0, 1.0);
        let level = (rms * 4.5).clamp(0.0, 1.0);

        let raw_bands = [bass, low_mid, high_mid, level];
        for i in 0..4 {
            let alpha = if raw_bands[i] > smooth_bands[i] { 0.45 } else { 0.18 };
            smooth_bands[i] = smooth_bands[i] * (1.0 - alpha) + raw_bands[i] * alpha;
        }

        // Beat detection on bass + level impulse.
        let instant_energy = bass * 0.7 + level * 0.3;
        energy_ema = energy_ema * 0.92 + instant_energy * 0.08;
        let ratio = if energy_ema > 0.005 {
            (instant_energy - energy_ema * 1.25).max(0.0) * 3.0
        } else {
            0.0
        };
        smooth_beat = (smooth_beat * 0.75).max(ratio.clamp(0.0, 1.0));

        // ------------------------------------------------------------------
        // Real-time Acoustic Vocal Formant (F1 / F2) & Viseme Extraction
        // ------------------------------------------------------------------
        // Sub-bass kick bleed (bins 1..5: 43..215 Hz)
        let sub_kick = avg_slice(&mags[1..5]) * 20.0;
        // F1 close vowels (I, U): ~300..520 Hz (bins 7..12)
        let f1_close = avg_slice(&mags[7..12]) * 38.0;
        // F1 open vowels (A, E, O): ~520..950 Hz (bins 12..22)
        let f1_open = avg_slice(&mags[12..22]) * 44.0;
        // F2 back/rounded vowels (U, O, A): ~950..1460 Hz (bins 22..34)
        let f2_back = avg_slice(&mags[22..34]) * 48.0;
        // F2 front/spread vowels (I, E, Ü): ~1500..2850 Hz (bins 35..66)
        let f2_front = avg_slice(&mags[35..66]) * 58.0;
        // Sibilants / Fricatives (S, Ş, Ç, Z, T): ~3600..7500 Hz (bins 84..174)
        let sib_energy = avg_slice(&mags[84..174]) * 75.0;

        // Harmonic peakiness (crest factor) in the core vocal band (bins 7..66):
        // Human singing/speech has strong formant peaks relative to flat broadband accompaniment.
        let vocal_slice = &mags[7..66];
        let vocal_mean = avg_slice(vocal_slice).max(1e-6);
        let vocal_peak = vocal_slice.iter().copied().fold(0.0f32, f32::max);
        let crest = ((vocal_peak / vocal_mean) - 1.65).clamp(0.0, 4.0) / 4.0;

        // Raw vocal energy suppressing pure sub-bass kick hits
        let vocal_raw = ((f1_close * 0.25 + f1_open * 0.40 + f2_back * 0.18 + f2_front * 0.22)
            * (0.55 + crest * 0.65)
            - sub_kick * 0.14)
            .max(0.0);

        // Syllabic articulation envelope:
        // Fast envelope tracks syllables (~30ms); slow floor tracks continuous accompaniment (~450ms).
        let atk = if vocal_raw > vocal_fast_env { 0.62 } else { 0.34 };
        vocal_fast_env = vocal_fast_env * (1.0 - atk) + vocal_raw * atk;
        let fl_alpha = if vocal_raw < vocal_floor_env { 0.22 } else { 0.045 };
        vocal_floor_env = vocal_floor_env * (1.0 - fl_alpha) + vocal_raw * fl_alpha;

        // Positive syllable onset flux + modulated vocal carrier
        let onset_flux = (vocal_raw - prev_vocal_raw).max(0.0) * 1.8;
        prev_vocal_raw = vocal_raw;

        // Subtract steady instrumental floor so the mouth closes between sung syllables
        let syllabic = ((vocal_fast_env - vocal_floor_env * 0.68).max(0.0) * 2.2
            + vocal_fast_env * crest * 0.55
            + onset_flux)
            .clamp(0.0, 1.0);

        // Sibilance (S, Ş, Ç, Z, T): high 3.6-7.5 kHz + elevated zero-crossing rate
        let raw_sib = if rms > 0.004 {
            ((sib_energy * 0.7 + ((zcr - 0.10) * 3.5).clamp(0.0, 1.0) * 0.3)
                * (1.0 - (sub_kick * 0.8).clamp(0.0, 0.7)))
            .clamp(0.0, 1.0)
        } else {
            0.0
        };

        // Jaw opening ratio: open vowels (f1_open) open wide; close vowels (f1_close) or sibilants stay narrower
        let open_vowel_ratio = (f1_open / (f1_open + f1_close * 0.85 + 1e-4)).clamp(0.25, 1.0);
        let raw_mouth_open = if rms > 0.0035 {
            (syllabic * (0.45 + open_vowel_ratio * 0.75) * (1.0 - raw_sib * 0.45)).clamp(0.0, 1.0)
        } else {
            0.0
        };

        // Lip form (-1.0 = rounded O/U, 0.0 = open A, +1.0 = wide spread I/E)
        let front_vs_back = (f2_front - (f2_back * 0.85 + f1_close * 0.35))
            / (f2_front + f2_back + f1_close * 0.35 + 1e-4);
        let raw_mouth_form = (front_vs_back * 1.85 + raw_sib * 0.35).clamp(-1.0, 1.0);
        let vocal_act = (syllabic * 1.3 + raw_sib * 0.5).clamp(0.0, 1.0);

        let target_mouth = [raw_mouth_open, raw_mouth_form, raw_sib, vocal_act];
        let mouth_alphas = [
            if target_mouth[0] > smooth_mouth[0] { 0.65 } else { 0.38 },
            0.35,
            0.50,
            0.30,
        ];
        for i in 0..4 {
            smooth_mouth[i] =
                smooth_mouth[i] * (1.0 - mouth_alphas[i]) + target_mouth[i] * mouth_alphas[i];
        }

        // 64 logarithmic spectrum bars across bins 1..256.
        for bar in 0..64 {
            let t0 = (bar as f32) / 64.0;
            let t1 = ((bar + 1) as f32) / 64.0;
            let b0 = (1.0 + (240.0f32).powf(t0)) as usize;
            let b1 = ((1.0 + (240.0f32).powf(t1)) as usize).max(b0 + 1).min(half - 1);
            let v = (avg_slice(&mags[b0..b1]) * (20.0 + bar as f32 * 0.6)).clamp(0.0, 1.0);
            let alpha = if v > smooth_spec[bar] { 0.5 } else { 0.2 };
            smooth_spec[bar] = smooth_spec[bar] * (1.0 - alpha) + v * alpha;
        }

        if let Ok(mut guard) = lock.lock() {
            guard.snapshot = AudioSnapshot {
                bands: smooth_bands,
                spectrum: smooth_spec,
                beat: smooth_beat,
                mouth: smooth_mouth,
                desktop_playing: rms > 0.003,
            };
        }
    }
}

fn avg_slice(slice: &[f32]) -> f32 {
    if slice.is_empty() {
        return 0.0;
    }
    let sum: f32 = slice.iter().sum();
    sum / (slice.len() as f32)
}

pub(crate) use crate::fft::fft_in_place;


#[allow(dead_code)]
fn _cstr_unused(_: &CStr) {}
