//! アプリ API: 音声入力（マイクの生音声の取得）。文字起こし（音声認識）は含まない。認識はアプリ側で行う。
//!
//! # 方式
//!
//! 取得は **Rust 側**（Windows は cpal / WASAPI）で行い、PCM を `app-audio-event` でアプリの iframe へ push する。
//! iframe では getUserMedia を使わせない。opaque origin（`sandbox="allow-scripts"`）では
//! `SecurityError: Invalid security origin` で失敗し、sandbox を緩めると許可が WebView2 のプロファイルに保存されて
//! 関門を Rust の 1 箇所に置けなくなるため（#395 の実機検証）。
//!
//! # 関門と上限
//!
//! - 呼び出し元インスタンスは webview のウィンドウラベルから決める（`app_instance::context_for_label`）。
//!   承認済みの権限セット（grant の `resolved.audio`）だけを見る。アーティファクト JSON の manifest は見ない。
//!   **追加する全コマンドで `require_instance_window`**。
//! - 1 インスタンス 1 ストリーム、全体で `MAX_TOTAL_STREAMS` 本まで。1 回の最大秒数は `maxSeconds`（host が数える）。
//!   `start` で指定できるレートは `sampleRates` の中だけ。拒否・上限超過はデバッグログへ「拒否: audio.<api> — …」で残す。
//! - 音声はメモリ上だけで流し、ファイルには書かない。ログに残すのは秒数・理由・デバイス名だけ。
//!
//! # 止まるとき
//!
//! アプリの `stop`、`maxSeconds` への到達、リロード（再承認の承認後を含む）、インスタンスの停止（承認の取り消し・
//! ウィンドウを閉じる・アーティファクトの削除を含む）、起動画面の「録音を止める」、デバイスのエラー。
//! 止まったら `{ kind: "ended", reason }` を 1 回だけ送る。**インスタンスの停止とリロードでは送らない**
//! （送り先の iframe が居なくなる・入れ替わる。新しい iframe が古いストリームの終了を自分のものと取り違えない）。
//! リロードでは、新しい iframe が `start` し直す。`start` がアプリへ失敗を返したストリームも、何も送らない。
//! `maxSeconds` は送ったサンプル数で数え、壁時計の期限（+2 秒）も持つ（デバイスが黙り込んでも host が強制する）。
//! 録音中は起動画面に「● 録音中」を出し、実行ウィンドウとサブウィンドウのタイトルに `● 録音中 — ` を付ける。
//! Web ビューア（HTTP）には出さない。
//!
//! # PCM の形式
//!
//! モノラル・符号付き 16 ビット・リトルエンディアン（`i16` LE）を base64 にして、約 100ms ごとに送る。
//! 入力が複数チャンネルなら平均する。レートは承認された範囲でアプリが選ぶ（既定は先頭）。

use crate::app_grant::AppGrantStore;
use crate::app_manifest::AudioPermission;
use base64::Engine;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

/// 全インスタンスを通した同時ストリーム数の上限。
pub const MAX_TOTAL_STREAMS: usize = 2;
/// 1 チャンクの長さ（ミリ秒）。
const CHUNK_MS: u32 = 100;
/// 録音中に実行ウィンドウ・サブウィンドウのタイトルの先頭へ付ける印。
pub const RECORDING_TITLE_PREFIX: &str = "● 録音中 — ";
/// `start` がデバイスを開く結果を待つ上限。
const OPEN_TIMEOUT: Duration = Duration::from_secs(5);

type Key = (String, String);

// ─── 終了の理由 ───────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    /// アプリが `audio.stop()` を呼んだ
    Stop,
    /// 起動画面の「録音を止める」
    UserStop,
    /// `maxSeconds` に達した
    MaxSeconds,
    /// ホットリロード（再承認の承認後を含む）。アプリへは `ended` を送らない（ログにだけ残る）
    Reload,
    /// インスタンスの停止（承認の取り消し・ウィンドウを閉じる・削除を含む）。アプリへは `ended` を送らない
    Stopped,
    /// デバイスのエラー（抜かれた・使用中になったなど）
    Error,
}

impl EndReason {
    pub fn as_str(self) -> &'static str {
        match self {
            EndReason::Stop => "stop",
            EndReason::UserStop => "userStop",
            EndReason::MaxSeconds => "maxSeconds",
            EndReason::Reload => "reload",
            EndReason::Stopped => "stopped",
            EndReason::Error => "error",
        }
    }
}

/// 止める合図と理由。**理由は最初の 1 回だけ**記録する（先に成立したものが勝つ）。
#[derive(Default)]
struct Shared {
    stop: AtomicBool,
    /// `start` がアプリへ失敗を返した（あるいは停止済みで返せなかった）ストリーム。アプリは streamId を知らないので、
    /// 後から `data` / `ended` を送らない
    silent: AtomicBool,
    reason: Mutex<Option<EndReason>>,
}

impl Shared {
    fn request_stop(&self, reason: EndReason) {
        if let Ok(mut g) = self.reason.lock() {
            if g.is_none() {
                *g = Some(reason);
            }
        }
        self.stop.store(true, Ordering::SeqCst);
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    fn mark_silent(&self) {
        self.silent.store(true, Ordering::SeqCst);
    }

    fn is_silent(&self) -> bool {
        self.silent.load(Ordering::SeqCst)
    }

    fn reason(&self) -> Option<EndReason> {
        self.reason.lock().ok().and_then(|g| *g)
    }
}

// ─── レジストリ（純粋な状態遷移。AppHandle を取らないのでテストできる） ─────────

struct StreamHandle {
    stream_id: u64,
    key: Key,
    shared: Arc<Shared>,
}

#[derive(Default)]
struct Inner {
    streams: HashMap<String, StreamHandle>,
    seq: u64,
}

impl Inner {
    /// 枠を確保する。1 インスタンス 1 ストリームと全体の上限はここで拒否する。
    fn reserve(&mut self, label: &str, key: Key, shared: Arc<Shared>) -> Result<u64, String> {
        if self.streams.contains_key(label) {
            return Err("このアプリは既に録音中です（1 つのアプリが同時に録音できるのは 1 本だけです）".to_string());
        }
        if self.streams.len() >= MAX_TOTAL_STREAMS {
            return Err(format!(
                "同時に録音できるのは全体で {} 本までです。どれかを止めてから開始してください",
                MAX_TOTAL_STREAMS
            ));
        }
        self.seq += 1;
        let stream_id = self.seq;
        self.streams.insert(label.to_string(), StreamHandle { stream_id, key, shared });
        Ok(stream_id)
    }

    fn take(&mut self, label: &str) -> Option<StreamHandle> {
        self.streams.remove(label)
    }

    /// 自分の（`stream_id` が一致する）枠だけを外す。古いスレッドが新しいストリームの枠を消さないため。
    fn remove_if(&mut self, label: &str, stream_id: u64) -> bool {
        match self.streams.get(label) {
            Some(h) if h.stream_id == stream_id => {
                self.streams.remove(label);
                true
            }
            _ => false,
        }
    }

    fn is_recording(&self, key: &Key) -> bool {
        self.streams.values().any(|h| &h.key == key)
    }
}

#[derive(Default)]
pub struct AppAudioRegistry(Mutex<Inner>);

impl AppAudioRegistry {
    /// 起動画面の状態（`recording`）用。
    pub fn is_recording(&self, worktree_id: &str, artifact_id: &str) -> bool {
        let key: Key = (worktree_id.to_string(), artifact_id.to_string());
        self.0.lock().map(|i| i.is_recording(&key)).unwrap_or(false)
    }

    fn take(&self, label: &str) -> Option<StreamHandle> {
        self.0.lock().ok().and_then(|mut i| i.take(label))
    }
}

// ─── PCM の変換（純関数。テストできる） ───────────────────────────────────

/// 入力（インターリーブの f32）を、モノラル・目的のレート・`i16` へ変換する。
///
/// - 複数チャンネルは平均する（片側だけが無音のマイクでも音量が半分になるだけで、歪まない）。
/// - レートの変換は面積平均（箱型フィルタ）。下げる方向（48k → 16k など）では折り返しを抑える。
///   上げる方向は零次ホールドで、音声認識の入力用途では使わない想定。
/// - 端数のフレーム（チャンネル数で割り切れない末尾）は捨てる。cpal はフレーム単位で渡す。
pub struct PcmConverter {
    channels: usize,
    /// 出力 1 サンプルが覆う入力サンプル数（`in_rate / out_rate`）
    step: f64,
    acc: f64,
    filled: f64,
}

impl PcmConverter {
    pub fn new(in_rate: u32, channels: u16, out_rate: u32) -> Self {
        Self {
            channels: channels.max(1) as usize,
            step: in_rate.max(1) as f64 / out_rate.max(1) as f64,
            acc: 0.0,
            filled: 0.0,
        }
    }

    pub fn push(&mut self, interleaved: &[f32], out: &mut Vec<i16>) {
        for frame in interleaved.chunks_exact(self.channels) {
            let x = (frame.iter().map(|v| *v as f64).sum::<f64>()) / self.channels as f64;
            let mut remaining = 1.0f64;
            while remaining > 1e-12 {
                let need = self.step - self.filled;
                if remaining + 1e-12 >= need {
                    self.acc += x * need;
                    out.push(to_i16((self.acc / self.step) as f32));
                    self.acc = 0.0;
                    self.filled = 0.0;
                    remaining -= need;
                } else {
                    self.acc += x * remaining;
                    self.filled += remaining;
                    remaining = 0.0;
                }
            }
        }
    }
}

fn to_i16(v: f32) -> i16 {
    (v.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

/// 変換後のサンプルを一定の長さに区切り、`maxSeconds` で打ち切る。
pub struct Chunker {
    chunk_samples: usize,
    limit_samples: u64,
    emitted: u64,
    buf: Vec<i16>,
}

impl Chunker {
    pub fn new(rate: u32, max_seconds: u32) -> Self {
        Self {
            chunk_samples: ((rate as u64 * CHUNK_MS as u64) / 1000).max(1) as usize,
            limit_samples: rate as u64 * max_seconds as u64,
            emitted: 0,
            buf: Vec::new(),
        }
    }

    /// サンプルを足し、送れるチャンクを `out` へ出す。上限に達したら `true`（超えた分は捨てる）。
    pub fn push(&mut self, samples: &[i16], out: &mut Vec<Vec<i16>>) -> bool {
        let room = self.limit_samples.saturating_sub(self.emitted + self.buf.len() as u64) as usize;
        let take = samples.len().min(room);
        self.buf.extend_from_slice(&samples[..take]);
        while self.buf.len() >= self.chunk_samples {
            let rest = self.buf.split_off(self.chunk_samples);
            let chunk = std::mem::replace(&mut self.buf, rest);
            self.emitted += chunk.len() as u64;
            out.push(chunk);
        }
        self.emitted + self.buf.len() as u64 >= self.limit_samples
    }

    /// 端数（1 チャンクに満たない残り）を取り出す。
    pub fn flush(&mut self) -> Option<Vec<i16>> {
        if self.buf.is_empty() {
            return None;
        }
        let chunk = std::mem::take(&mut self.buf);
        self.emitted += chunk.len() as u64;
        Some(chunk)
    }

    pub fn emitted(&self) -> u64 {
        self.emitted
    }
}

fn encode_pcm(samples: &[i16]) -> String {
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for s in samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

// ─── キャプチャのバックエンド（cfg で分けるのはここだけ） ───────────────────────

/// 開いた入力の情報。
struct CaptureInfo {
    device: String,
    rate: u32,
    channels: u16,
}

#[cfg(windows)]
mod backend {
    use super::CaptureInfo;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{FromSample, Sample, SizedSample};
    use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
    use std::sync::{Arc, Mutex};

    /// cpal の `Stream` は `Send` ではないので、開いたスレッドの中で持ち続ける。
    pub struct Capture {
        _stream: cpal::Stream,
        pub rx: Receiver<Vec<f32>>,
        error: Arc<Mutex<Option<String>>>,
        pub info: CaptureInfo,
    }

    impl Capture {
        pub fn error(&self) -> Option<String> {
            self.error.lock().ok().and_then(|g| g.clone())
        }
    }

    fn build<T>(
        dev: &cpal::Device,
        cfg: &cpal::StreamConfig,
        tx: SyncSender<Vec<f32>>,
        error: Arc<Mutex<Option<String>>>,
    ) -> Result<cpal::Stream, cpal::BuildStreamError>
    where
        T: SizedSample + Send + 'static,
        f32: FromSample<T>,
    {
        dev.build_input_stream(
            cfg,
            move |data: &[T], _| {
                let v: Vec<f32> = data.iter().map(|s| f32::from_sample(*s)).collect();
                // 詰まったら捨てる（コールバックを止めない）
                let _ = tx.try_send(v);
            },
            move |e| {
                if let Ok(mut g) = error.lock() {
                    *g = Some(e.to_string());
                }
            },
            None,
        )
    }

    /// 既定の入力デバイスを開いて録り始める。
    pub fn open() -> Result<Capture, String> {
        let host = cpal::default_host();
        let dev = host
            .default_input_device()
            .ok_or_else(|| "マイク（入力デバイス）が見つかりません".to_string())?;
        let cfg = dev
            .default_input_config()
            .map_err(|e| format!("マイクの設定を取得できません: {}", e))?;
        let device = dev.name().unwrap_or_else(|_| "（名前不明）".to_string());
        let info = CaptureInfo { device, rate: cfg.sample_rate().0, channels: cfg.channels() };
        let stream_cfg: cpal::StreamConfig = cfg.clone().into();
        let (tx, rx) = sync_channel::<Vec<f32>>(256);
        let error = Arc::new(Mutex::new(None));
        let stream = match cfg.sample_format() {
            cpal::SampleFormat::F32 => build::<f32>(&dev, &stream_cfg, tx, error.clone()),
            cpal::SampleFormat::I16 => build::<i16>(&dev, &stream_cfg, tx, error.clone()),
            cpal::SampleFormat::U16 => build::<u16>(&dev, &stream_cfg, tx, error.clone()),
            cpal::SampleFormat::I32 => build::<i32>(&dev, &stream_cfg, tx, error.clone()),
            f => return Err(format!("このマイクのサンプル形式 {:?} には対応していません", f)),
        }
        .map_err(|e| format!("マイクを開けませんでした: {}", e))?;
        stream.play().map_err(|e| format!("マイクの録音を開始できませんでした: {}", e))?;
        Ok(Capture { _stream: stream, rx, error, info })
    }
}

#[cfg(not(windows))]
mod backend {
    use super::CaptureInfo;
    use std::sync::mpsc::Receiver;

    pub struct Capture {
        pub rx: Receiver<Vec<f32>>,
        pub info: CaptureInfo,
    }

    impl Capture {
        pub fn error(&self) -> Option<String> {
            None
        }
    }

    pub fn open() -> Result<Capture, String> {
        Err("マイクの音声入力は Windows でだけ使えます".to_string())
    }
}

// ─── 呼び出し元の検査 ──────────────────────────────────────────────────

/// 呼び出し元の身元と、承認済みの `audio` 権限。権限が無ければ拒否してデバッグログへ残す。
fn caller(
    window: &tauri::Window,
    app: &AppHandle,
    what: &str,
) -> Result<(crate::app_instance::InstanceContext, AudioPermission), String> {
    crate::app_instance::require_instance_window(window)?;
    let ctx = crate::app_instance::context_for_label(app, window.label())?;
    let perm = app
        .state::<AppGrantStore>()
        .get(&ctx.worktree_id, &ctx.artifact_id)
        .and_then(|g| g.resolved.audio);
    match perm {
        Some(p) => Ok((ctx, p)),
        None => {
            let e = "このアプリにはマイクの権限がありません（manifest の audio）".to_string();
            denied(app, &ctx, what, e)
        }
    }
}

fn denied<T>(
    app: &AppHandle,
    ctx: &crate::app_instance::InstanceContext,
    what: &str,
    e: String,
) -> Result<T, String> {
    let note = if ctx.reapproval_pending { "（再承認待ちのため、新しい権限はまだ使えません）" } else { "" };
    crate::app_instance::add_log(
        app,
        &ctx.worktree_id,
        &ctx.artifact_id,
        "warn",
        format!("拒否: audio.{} — {}{}", what, e, note),
    );
    Err(e)
}

// ─── 録音中の表示 ──────────────────────────────────────────────────────

/// タイトルの先頭に `● 録音中 — ` を付け外しする（現在のタイトルへの接頭辞なので、もとの形式は変えない）。
fn apply_title(current: &str, recording: bool) -> String {
    let base = current.strip_prefix(RECORDING_TITLE_PREFIX).unwrap_or(current);
    if recording {
        format!("{}{}", RECORDING_TITLE_PREFIX, base)
    } else {
        base.to_string()
    }
}

/// 録音の状態に合わせて、実行ウィンドウとサブウィンドウのタイトルの印を付け外しする。
/// サブウィンドウを開いた・タイトルを変えた直後にも呼ぶ（アプリが印を消せないように）。
pub fn sync_titles(app: &AppHandle, instance_label: &str, key_worktree: &str, key_artifact: &str) {
    let recording = app.state::<AppAudioRegistry>().is_recording(key_worktree, key_artifact);
    apply_titles(app, instance_label, recording);
}

/// 録音の状態が変わったので、タイトルと起動画面の状態を更新する。
fn refresh_recording(app: &AppHandle, instance_label: &str, key: &Key) {
    sync_titles(app, instance_label, &key.0, &key.1);
    crate::app_instance::emit_changed(app, &key.0, &key.1);
}

fn apply_titles(app: &AppHandle, instance_label: &str, recording: bool) {
    let labels = std::iter::once(instance_label.to_string())
        .chain(crate::app_subwindow::labels_for_instance(app, instance_label));
    for label in labels {
        let Some(w) = app.get_webview_window(&label) else { continue };
        let Ok(title) = w.title() else { continue };
        let next = apply_title(&title, recording);
        if next != title {
            let _ = w.set_title(&next);
        }
    }
}

// ─── ストリーム ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioStartInfo {
    pub stream_id: u64,
    pub sample_rate: u32,
    pub max_seconds: u32,
}

struct StreamParams {
    app: AppHandle,
    label: String,
    key: Key,
    stream_id: u64,
    rate: u32,
    max_seconds: u32,
    shared: Arc<Shared>,
}

fn emit_event(app: &AppHandle, label: &str, payload: serde_json::Value) {
    crate::app_subwindow::emit_to_instance(app, label, "app-audio-event", payload);
}

/// 止めるまで PCM を送り続ける。戻り値は (終了の理由, 送った音声の秒数, デバイスのエラー)。
/// `capture` はここで手放す（マイクの使用中表示を、後始末より先に消す）。
fn run_loop(p: &StreamParams, capture: backend::Capture, started: Instant) -> (EndReason, f64, Option<String>) {
    let mut conv = PcmConverter::new(capture.info.rate, capture.info.channels, p.rate);
    let mut chunker = Chunker::new(p.rate, p.max_seconds);
    let mut seq: u64 = 0;
    let mut pcm: Vec<i16> = Vec::new();
    let mut chunks: Vec<Vec<i16>> = Vec::new();
    // サンプル数で数えた上限（正確）に加えて、壁時計の期限も持つ。デバイスが黙り込んでサンプルが届かなくても、
    // maxSeconds を host が強制できるように（余裕は処理の遅れの分）
    let wall_limit = Duration::from_secs(p.max_seconds as u64) + Duration::from_secs(2);

    let send = |chunks: &mut Vec<Vec<i16>>, seq: &mut u64| {
        if p.shared.is_silent() {
            chunks.clear();
            return;
        }
        for c in chunks.drain(..) {
            *seq += 1;
            emit_event(
                &p.app,
                &p.label,
                serde_json::json!({ "kind": "data", "streamId": p.stream_id, "seq": *seq, "data": encode_pcm(&c) }),
            );
        }
    };

    let mut device_error: Option<String> = None;
    while !p.shared.stopped() {
        match capture.rx.recv_timeout(Duration::from_millis(100)) {
            Ok(buf) => {
                pcm.clear();
                conv.push(&buf, &mut pcm);
                let reached = chunker.push(&pcm, &mut chunks);
                send(&mut chunks, &mut seq);
                if reached {
                    p.shared.request_stop(EndReason::MaxSeconds);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                device_error = Some("入力が途切れました".to_string());
                p.shared.request_stop(EndReason::Error);
            }
        }
        if device_error.is_none() {
            if let Some(e) = capture.error() {
                device_error = Some(e);
                p.shared.request_stop(EndReason::Error);
            }
        }
        if started.elapsed() >= wall_limit {
            p.shared.request_stop(EndReason::MaxSeconds);
        }
    }

    let reason = p.shared.reason().unwrap_or(EndReason::Error);
    // マイクを先に手放す（録音の表示が消えるのを待たせない）
    drop(capture);
    // 端数を送る。送り先の iframe が居なくなる（インスタンスの停止・リロード）ときは送らない
    if !matches!(reason, EndReason::Stopped | EndReason::Reload) {
        if let Some(rest) = chunker.flush() {
            chunks.push(rest);
            send(&mut chunks, &mut seq);
        }
    }
    (reason, chunker.emitted() as f64 / p.rate.max(1) as f64, device_error)
}

/// ストリームの後始末。**枠を先に空け**（アプリが `ended` の中で `start` し直せるように）、ログと表示を更新して、
/// 最後に `ended` を送る。パニックしたときもここを通る（枠と「録音中」の表示を残さない）。
fn finish_stream(p: &StreamParams, reason: EndReason, secs: f64, wall: f64, device_error: Option<String>) {
    // 自分の枠のときだけ外す（古いスレッドが、新しいストリームの枠を消さない）
    let removed = p
        .app
        .state::<AppAudioRegistry>()
        .0
        .lock()
        .map(|mut i| i.remove_if(&p.label, p.stream_id))
        .unwrap_or(false);
    let detail = match &device_error {
        Some(e) => format!("、デバイスのエラー: {}", e),
        None => String::new(),
    };
    crate::app_instance::add_log(
        &p.app,
        &p.key.0,
        &p.key.1,
        if reason == EndReason::Error { "warn" } else { "info" },
        format!(
            "録音を終了しました（stream {}、理由 {}、送信した音声 {:.1} 秒、経過 {:.1} 秒{}）",
            p.stream_id,
            reason.as_str(),
            secs,
            wall,
            detail
        ),
    );
    if removed {
        refresh_recording(&p.app, &p.label, &p.key);
    }
    // インスタンスの停止・リロードでは、送り先の iframe が入れ替わる・無くなるので送らない（新しい iframe が
    // 古いストリームの終了を自分のものと取り違えない）。`start` が失敗を返したストリームも、アプリは知らない
    if !matches!(reason, EndReason::Stopped | EndReason::Reload) && !p.shared.is_silent() {
        emit_event(
            &p.app,
            &p.label,
            serde_json::json!({ "kind": "ended", "streamId": p.stream_id, "reason": reason.as_str() }),
        );
    }
}

/// 専用スレッド。開いた結果を `open_tx` へ返し、以後は止まるまで PCM を送り続ける。
fn capture_thread(p: StreamParams, open_tx: std::sync::mpsc::SyncSender<Result<String, String>>) {
    let capture = match backend::open() {
        Ok(c) => c,
        Err(e) => {
            let _ = open_tx.send(Err(e));
            return;
        }
    };
    let _ = open_tx.send(Ok(format!(
        "{}（入力 {}Hz / {}ch → {}Hz モノラル）",
        capture.info.device, capture.info.rate, capture.info.channels, p.rate
    )));

    let started = Instant::now();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_loop(&p, capture, started)));
    let (reason, secs, device_error) = match outcome {
        Ok(v) => v,
        Err(_) => {
            // マイクは unwind で手放される。枠と表示だけが残らないようにここで片付ける
            p.shared.request_stop(EndReason::Error);
            (EndReason::Error, 0.0, Some("録音スレッドが異常終了しました".to_string()))
        }
    };
    finish_stream(&p, reason, secs, started.elapsed().as_secs_f64(), device_error);
}

// ─── Tauri コマンド（appinst-* ウィンドウからだけ） ─────────────────────

/// 録音を始める。`sample_rate` を省くと、承認されたレートの先頭。
#[tauri::command]
pub async fn app_audio_start(
    window: tauri::Window,
    app: AppHandle,
    sample_rate: Option<u32>,
) -> Result<AudioStartInfo, String> {
    let (ctx, perm) = caller(&window, &app, "start")?;
    let rate = match sample_rate {
        None => perm.sample_rates.first().copied().unwrap_or(16000),
        Some(r) if perm.sample_rates.contains(&r) => r,
        Some(r) => {
            return denied(
                &app,
                &ctx,
                "start",
                format!("サンプルレート {} Hz は承認されていません（承認済み: {:?}）", r, perm.sample_rates),
            );
        }
    };
    let key: Key = (ctx.worktree_id.clone(), ctx.artifact_id.clone());
    let shared = Arc::new(Shared::default());
    let reserved = {
        let reg = app.state::<AppAudioRegistry>();
        let mut inner = reg.0.lock().map_err(|_| "レジストリのロックが壊れています".to_string())?;
        inner.reserve(&ctx.instance_label, key.clone(), shared.clone())
    };
    let stream_id = match reserved {
        Ok(id) => id,
        Err(e) => return denied(&app, &ctx, "start", e),
    };

    let (open_tx, open_rx) = std::sync::mpsc::sync_channel::<Result<String, String>>(1);
    let params = StreamParams {
        app: app.clone(),
        label: ctx.instance_label.clone(),
        key: key.clone(),
        stream_id,
        rate,
        max_seconds: perm.max_seconds,
        shared: shared.clone(),
    };
    let spawned = std::thread::Builder::new()
        .name(format!("app-audio-{}", stream_id))
        .spawn(move || capture_thread(params, open_tx));
    let release_reservation = |app: &AppHandle| {
        if let Ok(mut i) = app.state::<AppAudioRegistry>().0.lock() {
            i.remove_if(&ctx.instance_label, stream_id);
        }
    };
    if let Err(e) = spawned {
        release_reservation(&app);
        return fail_open(&app, &ctx, format!("録音のスレッドを作れませんでした: {}", e));
    }
    let opened = match tauri::async_runtime::spawn_blocking(move || open_rx.recv_timeout(OPEN_TIMEOUT)).await {
        Ok(v) => v,
        Err(e) => {
            // 待ちが壊れた。スレッドは走っているかもしれないので、止めて、アプリには何も送らせない
            shared.mark_silent();
            shared.request_stop(EndReason::Error);
            release_reservation(&app);
            return fail_open(&app, &ctx, format!("マイクを開く待ちが壊れました: {}", e));
        }
    };
    match opened {
        Ok(Ok(desc)) => {
            // 開く間に停止・リロードが入っていた。アプリへは成功を返さない（返しても終了の通知が先に届いて
            // 取り違える）。スレッドは理由どおりに後始末をするが、アプリは streamId を知らないので何も送らせない
            if shared.stopped() {
                shared.mark_silent();
                release_reservation(&app);
                return fail_open(&app, &ctx, "録音は開始の直後に停止されました（停止・リロード）".to_string());
            }
            crate::app_instance::add_log(
                &app,
                &ctx.worktree_id,
                &ctx.artifact_id,
                "info",
                format!("録音を開始しました（stream {}、最大 {} 秒、{}）", stream_id, perm.max_seconds, desc),
            );
            refresh_recording(&app, &ctx.instance_label, &key);
            Ok(AudioStartInfo { stream_id, sample_rate: rate, max_seconds: perm.max_seconds })
        }
        Ok(Err(e)) => {
            release_reservation(&app);
            fail_open(&app, &ctx, e)
        }
        Err(_) => {
            // 時間切れ。アプリには失敗を返すので、遅れて開いても何も送らせず、すぐ止める
            shared.mark_silent();
            shared.request_stop(EndReason::Error);
            release_reservation(&app);
            fail_open(&app, &ctx, "マイクを開くのに時間がかかりすぎました".to_string())
        }
    }
}

/// 権限は通ったが、デバイスを開けなかった。拒否ではないので別の見出しでログに残す。
fn fail_open<T>(app: &AppHandle, ctx: &crate::app_instance::InstanceContext, e: String) -> Result<T, String> {
    crate::app_instance::add_log(
        app,
        &ctx.worktree_id,
        &ctx.artifact_id,
        "error",
        format!("失敗: audio.start — {}", e),
    );
    Err(e)
}

/// 自分（インスタンス）の録音を止める。録音中でなければ `false`（エラーにしない）。
#[tauri::command]
pub async fn app_audio_stop(window: tauri::Window, app: AppHandle) -> Result<bool, String> {
    let (ctx, _perm) = caller(&window, &app, "stop")?;
    Ok(stop_label(&app, &ctx.instance_label, EndReason::Stop))
}

/// 起動画面の「録音を止める」。ビューアからだけ（アプリ自身が呼ぶ経路ではない）。
#[tauri::command]
pub async fn app_audio_stop_for_instance(
    window: tauri::Window,
    app: AppHandle,
    worktree_id: String,
    artifact_id: String,
) -> Result<bool, String> {
    crate::app_instance::require_viewer_or_main(&window)?;
    crate::validate_path_component(&worktree_id)?;
    crate::validate_path_component(&artifact_id)?;
    let Some(label) = crate::app_instance::label_for_key(&app, &worktree_id, &artifact_id) else {
        return Ok(false);
    };
    let stopped = stop_label(&app, &label, EndReason::UserStop);
    if stopped {
        crate::app_instance::add_log(&app, &worktree_id, &artifact_id, "info", "起動画面から録音を止めました");
    }
    Ok(stopped)
}

fn stop_label(app: &AppHandle, label: &str, reason: EndReason) -> bool {
    let Some(h) = app.state::<AppAudioRegistry>().take(label) else { return false };
    h.shared.request_stop(reason);
    refresh_recording(app, label, &h.key);
    true
}

// ─── ライフサイクル（app_instance から呼ぶ） ─────────────────────────────

/// インスタンスの停止（`app_instance::release`）。録音は必ず止める。
pub fn on_instance_released(app: &AppHandle, label: &str) {
    // 呼び出し元（stop_instance など）が状態の通知を出すので、ここでは枠を外して止めるだけ
    if let Some(h) = app.state::<AppAudioRegistry>().take(label) {
        h.shared.request_stop(EndReason::Stopped);
    }
}

/// ホットリロード（再承認の承認後を含む）。旧 iframe のストリームは止め、新しい iframe が `start` し直す。
pub fn on_reload(app: &AppHandle, label: &str) {
    stop_label(app, label, EndReason::Reload);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(a: &str) -> Key {
        ("wt".to_string(), a.to_string())
    }

    #[test]
    fn registry_allows_one_stream_per_instance_and_caps_the_total() {
        let mut r = Inner::default();
        let id1 = r.reserve("appinst-1", key("a"), Arc::new(Shared::default())).unwrap();
        // 同じインスタンスの 2 本目は拒否
        assert!(r.reserve("appinst-1", key("a"), Arc::new(Shared::default())).unwrap_err().contains("1 本だけ"));
        let id2 = r.reserve("appinst-2", key("b"), Arc::new(Shared::default())).unwrap();
        assert!(id2 > id1);
        // 全体の上限
        let e = r.reserve("appinst-3", key("c"), Arc::new(Shared::default())).unwrap_err();
        assert!(e.contains(&MAX_TOTAL_STREAMS.to_string()), "{e}");
        // 外せば空く
        assert!(r.take("appinst-1").is_some());
        assert!(r.reserve("appinst-3", key("c"), Arc::new(Shared::default())).is_ok());
    }

    #[test]
    fn remove_if_ignores_a_stale_stream_id() {
        let mut r = Inner::default();
        let old = r.reserve("appinst-1", key("a"), Arc::new(Shared::default())).unwrap();
        r.take("appinst-1");
        let new = r.reserve("appinst-1", key("a"), Arc::new(Shared::default())).unwrap();
        // 古いスレッドの後始末が、新しいストリームの枠を消してはいけない
        assert!(!r.remove_if("appinst-1", old));
        assert!(r.is_recording(&key("a")));
        assert!(r.remove_if("appinst-1", new));
        assert!(!r.is_recording(&key("a")));
    }

    #[test]
    fn is_recording_is_per_artifact() {
        let mut r = Inner::default();
        r.reserve("appinst-1", key("a"), Arc::new(Shared::default())).unwrap();
        assert!(r.is_recording(&key("a")));
        assert!(!r.is_recording(&key("b")));
        assert!(!r.is_recording(&("other".to_string(), "a".to_string())));
    }

    #[test]
    fn the_first_end_reason_wins() {
        let s = Shared::default();
        assert!(!s.stopped() && s.reason().is_none());
        s.request_stop(EndReason::MaxSeconds);
        s.request_stop(EndReason::Stop);
        s.request_stop(EndReason::Stopped);
        assert!(s.stopped());
        assert_eq!(s.reason(), Some(EndReason::MaxSeconds));
    }

    #[test]
    fn end_reasons_have_stable_names() {
        let all = [
            (EndReason::Stop, "stop"),
            (EndReason::UserStop, "userStop"),
            (EndReason::MaxSeconds, "maxSeconds"),
            (EndReason::Reload, "reload"),
            (EndReason::Stopped, "stopped"),
            (EndReason::Error, "error"),
        ];
        for (r, name) in all {
            assert_eq!(r.as_str(), name);
        }
    }

    #[test]
    fn converter_downmixes_and_decimates_by_area_average() {
        // 48k ステレオ → 16k モノラル。3 フレームごとに 1 サンプル（平均）
        let mut c = PcmConverter::new(48000, 2, 16000);
        let mut out = Vec::new();
        // 左 0.6 / 右 0.0 → モノラル 0.3 が 6 フレーム
        let input: Vec<f32> = (0..6).flat_map(|_| [0.6f32, 0.0]).collect();
        c.push(&input, &mut out);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|s| (*s as i32 - (0.3f32 * 32767.0).round() as i32).abs() <= 1), "{out:?}");
    }

    #[test]
    fn converter_keeps_state_across_pushes_and_does_not_drift() {
        // 44.1k → 16k（割り切れない比）。1 秒ぶんを細切れに入れても、出力はほぼ 16000 サンプル
        let mut whole = PcmConverter::new(44100, 1, 16000);
        let mut split = PcmConverter::new(44100, 1, 16000);
        let input: Vec<f32> = (0..44100).map(|i| ((i as f32) * 0.01).sin() * 0.5).collect();
        let (mut a, mut b) = (Vec::new(), Vec::new());
        whole.push(&input, &mut a);
        for part in input.chunks(441) {
            split.push(part, &mut b);
        }
        assert_eq!(a, b, "入力の区切り方で結果が変わってはいけない");
        assert!((a.len() as i32 - 16000).abs() <= 1, "{}", a.len());
    }

    #[test]
    fn converter_clamps_and_ignores_a_partial_frame() {
        let mut c = PcmConverter::new(16000, 2, 16000);
        let mut out = Vec::new();
        // 1 フレーム + 端数 1 サンプル。範囲外の値は ±32767 に収める
        c.push(&[2.0, 2.0, 5.0], &mut out);
        assert_eq!(out, vec![32767]);
        let mut out = Vec::new();
        PcmConverter::new(16000, 1, 16000).push(&[-3.0], &mut out);
        assert_eq!(out, vec![-32767]);
    }

    #[test]
    fn chunker_splits_into_100ms_chunks_and_flushes_the_rest() {
        let mut ch = Chunker::new(16000, 60);
        let mut out = Vec::new();
        // 16000Hz の 100ms は 1600 サンプル。4000 サンプル = 2 チャンク + 端数 800
        assert!(!ch.push(&vec![1i16; 4000], &mut out));
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|c| c.len() == 1600));
        let rest = ch.flush().unwrap();
        assert_eq!(rest.len(), 800);
        assert!(ch.flush().is_none());
        assert_eq!(ch.emitted(), 4000);
    }

    #[test]
    fn chunker_stops_exactly_at_max_seconds() {
        // 1 秒 = 16000 サンプル。1.5 秒ぶん入れても 1 秒で打ち切る
        let mut ch = Chunker::new(16000, 1);
        let mut out = Vec::new();
        let reached = ch.push(&vec![7i16; 24000], &mut out);
        assert!(reached);
        let flushed = ch.flush().map(|c| c.len()).unwrap_or(0) as u64;
        assert_eq!(ch.emitted(), 16000, "{flushed}");
        // 上限に達したあとに足しても増えない
        let mut out2 = Vec::new();
        assert!(ch.push(&[1, 2, 3], &mut out2));
        assert!(out2.is_empty());
        assert!(ch.flush().is_none());
    }

    #[test]
    fn pcm_is_encoded_as_little_endian_i16() {
        let s = encode_pcm(&[1, -2, 0x1234]);
        let bytes = base64::engine::general_purpose::STANDARD.decode(s).unwrap();
        assert_eq!(bytes, vec![0x01, 0x00, 0xFE, 0xFF, 0x34, 0x12]);
    }

    #[test]
    fn title_prefix_is_added_and_removed_idempotently() {
        let t = "Mic — oretachi app";
        let on = apply_title(t, true);
        assert_eq!(on, format!("{}{}", RECORDING_TITLE_PREFIX, t));
        // 二重に付かない
        assert_eq!(apply_title(&on, true), on);
        assert_eq!(apply_title(&on, false), t);
        assert_eq!(apply_title(t, false), t);
    }
}
