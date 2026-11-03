//! #421 診断用の PTY タップ（**dev ビルド限定**）。
//!
//! 「日本語入力中に文字・カーソルが欠ける」症状が、パイプラインのどの層
//! （Claude Code → ConPTY → xterm のパース/幅 → xterm の WebGL 描画）で起きているかを
//! 切り分けるため、ConPTY 出力・PTY 入力・リサイズを spawn 時点から JSONL へ記録する。
//!
//! 既存の出力リングバッファ（64KB）を使わないのは、Claude Code の再描画で症状の瞬間が
//! すぐ押し出されるうえ、途中から再生すると初期状態もリサイズ履歴も欠けて参照エミュレータの
//! 画面が合わなくなるため。
//!
//! 有効化は `cfg!(debug_assertions)` かつ環境変数 `ORETACHI_PTY_TAP=1`。release ビルドでは
//! `open` が常に `None` を返し、フックは呼ばれない。
//!
//! **注意: 入力（キー入力）も平文で記録する。** パスワード等を端末へ打つ操作もそのまま残るので、
//! 診断が済んだらタップファイル（`<app_log_dir>/pty-tap/`）を削除すること。
//!
//! 記録形式（1 行 1 レコード）:
//! - `{"t":ms,"d":"s","rows":R,"cols":C}`        開始（先頭の 1 行）
//! - `{"t":ms,"d":"o","off":N,"b64":"..."}`      ConPTY 出力（`off` は記録前の累積バイト数）
//! - `{"t":ms,"d":"i","b64":"..."}`              PTY 入力
//! - `{"t":ms,"d":"r","rows":R,"cols":C}`        リサイズ

// release ではフックが呼ばれず（`open` が常に None）、本体は使われない
#![cfg_attr(not(debug_assertions), allow(dead_code))]

use base64::Engine;
use serde_json::{json, Value};
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// セッションあたりの記録上限。超えたら記録を止める。
const MAX_TAP_BYTES: u64 = 64 * 1024 * 1024;

const MIN_ROWS: u16 = 4;
const MIN_COLS: u16 = 20;

struct TapInner {
    writer: BufWriter<File>,
    written: u64,
    stopped: bool,
}

pub struct PtyTap {
    path: PathBuf,
    session_id: u32,
    inner: Mutex<TapInner>,
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn b64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

impl PtyTap {
    /// タップを開く。無効（release / 環境変数なし）または失敗時は `None`。
    pub fn open(dir: &Path, session_id: u32, rows: u16, cols: u16) -> Option<Arc<PtyTap>> {
        if !cfg!(debug_assertions) {
            return None;
        }
        if std::env::var("ORETACHI_PTY_TAP").ok().as_deref() != Some("1") {
            return None;
        }
        if let Err(e) = std::fs::create_dir_all(dir) {
            log::warn!("[pty_tap] create_dir_all failed: {}", e);
            return None;
        }
        let path = dir.join(format!("{}-{}.jsonl", session_id, now_ms()));
        let file = match File::create(&path) {
            Ok(f) => f,
            Err(e) => {
                log::warn!("[pty_tap] create file failed: {}", e);
                return None;
            }
        };
        let tap = Arc::new(PtyTap {
            path: path.clone(),
            session_id,
            inner: Mutex::new(TapInner {
                writer: BufWriter::new(file),
                written: 0,
                stopped: false,
            }),
        });
        tap.append(json!({"t": now_ms() as u64, "d": "s", "rows": rows, "cols": cols}));
        log::info!("[pty_tap] session_id={} recording to {}", session_id, path.display());
        Some(tap)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn output(&self, off: u64, data: &[u8]) {
        self.append(json!({"t": now_ms() as u64, "d": "o", "off": off, "b64": b64(data)}));
    }

    pub fn input(&self, data: &[u8]) {
        self.append(json!({"t": now_ms() as u64, "d": "i", "b64": b64(data)}));
    }

    pub fn resize(&self, rows: u16, cols: u16) {
        self.append(json!({"t": now_ms() as u64, "d": "r", "rows": rows, "cols": cols}));
    }

    fn append(&self, record: Value) {
        let mut line = record.to_string();
        line.push('\n');
        let mut g = match self.inner.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        if g.stopped {
            return;
        }
        if g.written + line.len() as u64 > MAX_TAP_BYTES {
            g.stopped = true;
            log::warn!(
                "[pty_tap] session_id={} reached {} bytes; recording stopped",
                self.session_id,
                MAX_TAP_BYTES
            );
            return;
        }
        if g.writer.write_all(line.as_bytes()).and_then(|_| g.writer.flush()).is_err() {
            g.stopped = true;
            return;
        }
        g.written += line.len() as u64;
    }
}

/// 参照エミュレータ (vt100) の再生結果。
pub struct ReplayResult {
    pub rows: u16,
    pub cols: u16,
    /// 再生した出力の累積バイト数（`off + len` の最大値）。
    pub replayed_bytes: u64,
    /// `(row, col)`
    pub cursor: (u16, u16),
    /// カーソル行 ±`window` 行の `(行番号, 内容)`。
    pub lines: Vec<(u16, String)>,
    /// カーソル行の全セル `[文字, 幅]`（幅 0 = 全角の続きセル）。空セルは省略。
    pub cursor_row_cells: Vec<(u16, String, u8)>,
}

/// タップ（JSONL）を先頭から読み、vt100 に流して画面状態を返す。
///
/// `r` レコードで `set_size` を呼ぶので、リサイズを挟んでも寸法が追従する。
pub fn replay_tap<R: BufRead>(reader: R, window: u16) -> Result<ReplayResult, String> {
    let mut parser: Option<vt100::Parser> = None;
    let mut replayed: u64 = 0;
    for line in reader.lines() {
        let line = line.map_err(|e| format!("read error: {}", e))?;
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            // 書き込み途中の末尾行は捨てる
            Err(_) => continue,
        };
        let dims = |v: &Value| {
            (
                (v["rows"].as_u64().unwrap_or(24) as u16).max(MIN_ROWS),
                (v["cols"].as_u64().unwrap_or(80) as u16).max(MIN_COLS),
            )
        };
        match v["d"].as_str() {
            Some("s") => {
                let (r, c) = dims(&v);
                parser = Some(vt100::Parser::new(r, c, 0));
            }
            Some("r") => {
                if let Some(p) = parser.as_mut() {
                    let (r, c) = dims(&v);
                    p.screen_mut().set_size(r, c);
                }
            }
            Some("o") => {
                if let (Some(p), Some(s)) = (parser.as_mut(), v["b64"].as_str()) {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(s)
                        .map_err(|e| format!("base64 error: {}", e))?;
                    p.process(&bytes);
                    replayed = v["off"].as_u64().unwrap_or(replayed) + bytes.len() as u64;
                }
            }
            _ => {}
        }
    }
    let parser = parser.ok_or_else(|| "tap has no start record".to_string())?;
    let screen = parser.screen();
    let (rows, cols) = screen.size();
    let (cy, cx) = screen.cursor_position();
    let first = cy.saturating_sub(window);
    let last = (cy + window).min(rows.saturating_sub(1));
    let all_rows: Vec<String> = screen.rows(0, cols).collect();
    let lines = (first..=last)
        .map(|r| (r, all_rows.get(r as usize).cloned().unwrap_or_default()))
        .collect();
    let mut cells = Vec::new();
    for c in 0..cols {
        if let Some(cell) = screen.cell(cy, c) {
            let width = if cell.is_wide_continuation() {
                0
            } else if cell.is_wide() {
                2
            } else {
                1
            };
            let text = cell.contents();
            if !text.is_empty() || width == 0 {
                cells.push((c, text.to_string(), width));
            }
        }
    }
    Ok(ReplayResult {
        rows,
        cols,
        replayed_bytes: replayed,
        cursor: (cy, cx),
        lines,
        cursor_row_cells: cells,
    })
}

pub fn replay_file(path: &Path, window: u16) -> Result<ReplayResult, String> {
    let f = File::open(path).map_err(|e| format!("open tap failed: {}", e))?;
    replay_tap(BufReader::new(f), window)
}

/// 末尾 `max` バイトを、エスケープを可視化した文字列にする（`\x1b[12G` 形式）。
/// 先頭が UTF-8 の途中なら継続バイトを読み飛ばす。
pub fn visualize_tail(data: &[u8], max: usize) -> String {
    let mut start = data.len().saturating_sub(max);
    while start < data.len() && (data[start] & 0xC0) == 0x80 {
        start += 1;
    }
    let mut out = String::new();
    for ch in String::from_utf8_lossy(&data[start..]).chars() {
        match ch {
            '\x1b' => out.push_str("\\x1b"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n\n"),
            '\x07' => out.push_str("\\x07"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// 出力レコードだけをつなげた生バイト列（`visualize_tail` 用）。
pub fn read_output_bytes(path: &Path) -> Result<Vec<u8>, String> {
    let f = File::open(path).map_err(|e| format!("open tap failed: {}", e))?;
    let mut out = Vec::new();
    for line in BufReader::new(f).lines() {
        let Ok(line) = line else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        if v["d"].as_str() == Some("o") {
            if let Some(s) = v["b64"].as_str() {
                if let Ok(b) = base64::engine::general_purpose::STANDARD.decode(s) {
                    out.extend_from_slice(&b);
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn rec(d: &str, extra: Value) -> String {
        let mut v = json!({"t": 0, "d": d});
        for (k, val) in extra.as_object().unwrap() {
            v[k] = val.clone();
        }
        format!("{}\n", v)
    }

    fn out_rec(off: u64, bytes: &[u8]) -> String {
        rec("o", json!({"off": off, "b64": b64(bytes)}))
    }

    #[test]
    fn replays_fullwidth_text_with_cell_widths() {
        let mut tap = rec("s", json!({"rows": 10, "cols": 40}));
        let text = "あい\x1b[1Gx".as_bytes().to_vec();
        tap.push_str(&out_rec(0, &text));
        let r = replay_tap(Cursor::new(tap), 1).unwrap();
        assert_eq!(r.cursor, (0, 1));
        assert_eq!(r.replayed_bytes, text.len() as u64);
        // CSI 1G で先頭へ戻って x を上書きするので、あ(幅2) は x(幅1) + 空白に置き換わる
        assert_eq!(r.lines[0].1.chars().next(), Some('x'));
    }

    #[test]
    fn fullwidth_cells_have_width_two_and_continuation() {
        let mut tap = rec("s", json!({"rows": 10, "cols": 40}));
        tap.push_str(&out_rec(0, "あ".as_bytes()));
        let r = replay_tap(Cursor::new(tap), 0).unwrap();
        assert_eq!(r.cursor, (0, 2));
        assert_eq!(r.cursor_row_cells[0], (0, "あ".to_string(), 2));
        assert_eq!(r.cursor_row_cells[1], (1, String::new(), 0));
    }

    #[test]
    fn resize_record_changes_dimensions() {
        let mut tap = rec("s", json!({"rows": 10, "cols": 40}));
        tap.push_str(&out_rec(0, b"abc"));
        tap.push_str(&rec("r", json!({"rows": 12, "cols": 60})));
        let r = replay_tap(Cursor::new(tap), 0).unwrap();
        assert_eq!((r.rows, r.cols), (12, 60));
    }

    #[test]
    fn split_multibyte_across_records_is_decoded() {
        let bytes = "日".as_bytes();
        let mut tap = rec("s", json!({"rows": 10, "cols": 40}));
        tap.push_str(&out_rec(0, &bytes[..1]));
        tap.push_str(&out_rec(1, &bytes[1..]));
        let r = replay_tap(Cursor::new(tap), 0).unwrap();
        assert_eq!(r.cursor, (0, 2));
        assert_eq!(r.replayed_bytes, 3);
    }

    #[test]
    fn truncated_last_line_is_ignored() {
        let mut tap = rec("s", json!({"rows": 10, "cols": 40}));
        tap.push_str(&out_rec(0, b"ab"));
        tap.push_str("{\"t\":0,\"d\":\"o\",\"off\":2,\"b64\":\"YW");
        let r = replay_tap(Cursor::new(tap), 0).unwrap();
        assert_eq!(r.cursor, (0, 2));
    }

    #[test]
    fn visualize_tail_shows_escapes() {
        assert_eq!(visualize_tail(b"a\x1b[12Gb\r", 100), "a\\x1b[12Gb\\r");
    }
}
