//! アプリ形式アーティファクトのスケジューラー（#413）。
//!
//! manifest の `permissions.scheduler.jobs` に**静的に宣言**されたジョブを、アプリのインスタンスが
//! 起動中（Running）のあいだだけ、ホスト側が cron 式どおりに実行する。
//!
//! # 前提
//!
//! - ジョブの内容は承認ダイアログに出る内容と同じ。実行時は**保存済みの grant**（`resolved.scheduler`）
//!   だけを読み、アーティファクト JSON の manifest は見ない（AI が書き換えられるため）。
//! - 実行は既存の関門を通る。tool ジョブは `mcp_server::call_tool_for_app_instance`
//!   （`authorize_app_tool`）、aiTerminal ジョブは `ai_terminal::open_inner`（`maxConcurrent` / 全体上限）。
//!   スケジューラーが他の権限の迂回路にはならない。
//! - インスタンスごとに 1 タスク。停止（`release`）で止まり、再起動をまたいで復活しない。
//!   ホットリロードは `release` を通らないので、リロードをまたいで動き続ける。
//! - 時刻はローカル時刻の壁時計（`naive_local`）で評価する。長い sleep 1 回に頼らず、毎分の境目で現在時刻を
//!   引き直すので、PC のスリープ復帰でずれない。取りこぼした回は追いかけない（窓は直近
//!   [`CATCH_UP_WINDOW_MINUTES`] 分）。
//! - **夏時間は考慮しない**（壁時計の分をそのまま照合する）。秋の巻き戻しでは、繰り返される時間帯の日次ジョブが
//!   2 回走りうる。春の飛び越しでは、飛ばされた時間帯の日次ジョブはその日走らない。毎分・毎時のジョブには
//!   影響しない。Vixie cron のように重複・補完はしない。
//! - 同じジョブは同時に 1 回だけ。前回の実行が終わっていなければ、その回は飛ばしてログに残す。
//! - aiTerminal ジョブは、**そのジョブが前回開いた端末がまだ残っている間は次回を飛ばす**（端末は自動では
//!   閉じないので、周期が短いとアプリの `maxConcurrent` と全体上限を使い切って、他アプリの端末も開けなくなる）。
//!   端末が閉じられた（`aiTerminal.close`・ユーザーが閉じた・エージェントの終了）次の回から再開する。
//!   飛ばし始めと再開のときだけログに 1 行ずつ残す（毎分は出さない）。

use chrono::{Duration as ChronoDuration, Local, NaiveDateTime, Timelike};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager};

use crate::app_manifest::{SchedulerAction, SchedulerJob};

/// スリープ復帰などで tick が遅れたとき、遡って拾う最大の分数。これより前の回は取りこぼし扱い。
const CATCH_UP_WINDOW_MINUTES: i64 = 2;

// ─── cron（5 フィールド: 分 時 日 月 曜日。`*` `,` `-` `/` だけ） ─────────────────────

/// cron 式の空白を 1 つに正規化する（`requested_changed` が書き方の揺れで再承認を求めないように）。
pub fn normalize_cron(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronSpec {
    minutes: u64,
    hours: u32,
    /// bit 1..=31
    dom: u32,
    /// bit 1..=12
    months: u16,
    /// bit 0..=6（0 = 日曜。7 は 0 に畳む）
    dow: u8,
    /// フィールドが `*` で始まるか（Vixie cron の日・曜日の判定に使う）
    dom_star: bool,
    dow_star: bool,
}

fn parse_field(text: &str, min: u32, max: u32, name: &str) -> Result<u64, String> {
    let mut bits: u64 = 0;
    for part in text.split(',') {
        if part.is_empty() {
            return Err(format!("{} に空の要素があります", name));
        }
        let (range, step) = match part.split_once('/') {
            Some((r, st)) => {
                let st: u32 = st.parse().map_err(|_| format!("{} のステップ '{}' が数ではありません", name, st))?;
                if st == 0 {
                    return Err(format!("{} のステップは 1 以上にしてください", name));
                }
                (r, Some(st))
            }
            None => (part, None),
        };
        let num = |t: &str| -> Result<u32, String> {
            let n: u32 = t.parse().map_err(|_| format!("{} の '{}' は使えません（数字・* ・, ・- ・/ だけ）", name, t))?;
            if n < min || n > max {
                return Err(format!("{} の値 {} は {}〜{} の範囲で指定してください", name, n, min, max));
            }
            Ok(n)
        };
        let (lo, hi) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            let (a, b) = (num(a)?, num(b)?);
            if a > b {
                return Err(format!("{} の範囲 {}-{} は昇順で指定してください", name, a, b));
            }
            (a, b)
        } else {
            let a = num(range)?;
            // `a/n` は a から上限まで
            (a, if step.is_some() { max } else { a })
        };
        let step = step.unwrap_or(1);
        let mut v = lo;
        while v <= hi {
            bits |= 1u64 << v;
            v += step;
        }
    }
    Ok(bits)
}

impl CronSpec {
    pub fn parse(expr: &str) -> Result<Self, String> {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(format!("cron 式は 5 フィールド（分 時 日 月 曜日）で指定してください（{} フィールド）", fields.len()));
        }
        let minutes = parse_field(fields[0], 0, 59, "分")?;
        let hours = parse_field(fields[1], 0, 23, "時")? as u32;
        let dom = parse_field(fields[2], 1, 31, "日")? as u32;
        let months = parse_field(fields[3], 1, 12, "月")? as u16;
        let dow_raw = parse_field(fields[4], 0, 7, "曜日")?;
        // 7 は日曜（0）
        let dow = ((dow_raw & 0x7f) | ((dow_raw >> 7) & 1)) as u8;
        Ok(Self {
            minutes,
            hours,
            dom,
            months,
            dow,
            dom_star: fields[2].starts_with('*'),
            dow_star: fields[4].starts_with('*'),
        })
    }

    /// `t`（分未満は無視）が発火時刻か。
    pub fn matches(&self, t: &NaiveDateTime) -> bool {
        use chrono::Datelike;
        if self.minutes & (1u64 << t.minute()) == 0
            || self.hours & (1u32 << t.hour()) == 0
            || self.months & (1u16 << t.month()) == 0
        {
            return false;
        }
        let dom_ok = self.dom & (1u32 << t.day()) != 0;
        let dow_ok = self.dow & (1u8 << t.weekday().num_days_from_sunday()) != 0;
        // Vixie cron: 日・曜日の両方が制限されているときは「どちらか」、片方でも `*` 始まりなら「両方」
        if self.dom_star || self.dow_star {
            dom_ok && dow_ok
        } else {
            dom_ok || dow_ok
        }
    }

    /// `after` より後で `upto` 以前の分のうち、最初の発火時刻。窓は呼び出し側が数分に絞る前提。
    pub fn first_fire_in(&self, after: NaiveDateTime, upto: NaiveDateTime) -> Option<NaiveDateTime> {
        let mut m = after.with_second(0)?.with_nanosecond(0)? + ChronoDuration::minutes(1);
        // 窓は数分だが、念のため反復を打ち切る
        for _ in 0..(CATCH_UP_WINDOW_MINUTES * 4 + 8) {
            if m > upto {
                return None;
            }
            if self.matches(&m) {
                return Some(m);
            }
            m += ChronoDuration::minutes(1);
        }
        None
    }
}

// ─── ランナー ────────────────────────────────────────────────────────

/// インスタンスのラベル → スケジューラータスク。
#[derive(Default)]
pub struct AppSchedulerRegistry(Mutex<HashMap<String, tauri::async_runtime::JoinHandle<()>>>);

/// インスタンスが Running になったときに呼ぶ。同じラベルのタスクが残っていれば置き換える。
pub fn on_instance_started(app: &AppHandle, label: &str) {
    let app2 = app.clone();
    let label2 = label.to_string();
    let handle = tauri::async_runtime::spawn(async move { run_loop(app2, label2).await });
    if let Ok(mut m) = app.state::<AppSchedulerRegistry>().0.lock() {
        if let Some(old) = m.insert(label.to_string(), handle) {
            old.abort();
        }
    }
}

/// インスタンスの停止時に呼ぶ（`app_instance::release`）。
pub fn on_instance_released(app: &AppHandle, label: &str) {
    if let Ok(mut m) = app.state::<AppSchedulerRegistry>().0.lock() {
        if let Some(h) = m.remove(label) {
            h.abort();
        }
    }
}

/// aiTerminal ジョブの今回を飛ばすか。前回このジョブが開いた端末（`recorded`）が、いまも生きている
/// このインスタンスの端末（`live_ids`）に含まれていれば飛ばす。
fn skip_ai_job(recorded: Option<&str>, live_ids: &[String]) -> bool {
    recorded.map_or(false, |r| live_ids.iter().any(|l| l == r))
}

/// ランナーが持つジョブごとの状態。1 つのロックにまとめて、「端末の記録」と「実行中の印」を
/// 同時に更新する（別々にすると、その隙間で次の tick がどちらの歯止めにも当たらず二重に開く）。
#[derive(Default)]
struct JobState {
    /// 実行中のジョブ ID
    inflight: HashSet<String>,
    /// aiTerminal ジョブ ID → 直近に開いた terminal_id
    terminals: HashMap<String, String>,
    /// 端末が残っているため飛ばしている最中のジョブ ID（ログを遷移のときだけ出すため）
    skipping: HashSet<String>,
}

/// 次の分の境目までの待ち時間（1〜60 秒）。
fn sleep_until_next_minute(now: &NaiveDateTime) -> std::time::Duration {
    let into = now.second() as u64;
    std::time::Duration::from_secs((60 - into).clamp(1, 60))
}

async fn run_loop(app: AppHandle, label: String) {
    let state: Arc<Mutex<JobState>> = Arc::new(Mutex::new(JobState::default()));
    let key = |ctx: &crate::app_instance::InstanceContext| (ctx.worktree_id.clone(), ctx.artifact_id.clone());
    let mut last = Local::now().naive_local();
    loop {
        tokio::time::sleep(sleep_until_next_minute(&Local::now().naive_local())).await;
        let now = Local::now().naive_local();
        // 起動中でなくなっていれば終わる（`release` の abort より先に気づいた場合の保険）
        let Ok(ctx) = crate::app_instance::context_for_label(&app, &label) else { return };
        // 時計が戻った（夏時間の終わり・手動変更）ときは、その時点から数え直す
        let from = if now < last { now } else { last.max(now - ChronoDuration::minutes(CATCH_UP_WINDOW_MINUTES)) };
        last = now;

        // 毎回、保存済みの grant から引き直す。再承認が済めば次の tick から新しいジョブが効く
        let Some(grant) = app
            .state::<crate::app_grant::AppGrantStore>()
            .get(&ctx.worktree_id, &ctx.artifact_id)
        else {
            continue;
        };
        let Some(sch) = grant.resolved.scheduler.clone() else { continue };
        for job in sch.jobs {
            let Ok(spec) = CronSpec::parse(&job.cron) else {
                crate::app_instance::add_log(
                    &app,
                    &ctx.worktree_id,
                    &ctx.artifact_id,
                    "error",
                    format!("スケジューラー: ジョブ '{}' の cron 式を解釈できません: {}", job.id, job.cron),
                );
                continue;
            };
            if spec.first_fire_in(from, now).is_none() {
                continue;
            }
            // aiTerminal ジョブ: 前回開いた端末が残っていれば今回は飛ばす（端末の生死は PTY の生存で判定）
            if matches!(job.action, SchedulerAction::AiTerminal { .. }) {
                let recorded = state.lock().ok().and_then(|s| s.terminals.get(&job.id).cloned());
                if recorded.is_some() {
                    let live: Vec<String> = crate::ai_terminal::list_for(&app, &key(&ctx))
                        .into_iter()
                        .map(|t| t.terminal_id)
                        .collect();
                    let skip = skip_ai_job(recorded.as_deref(), &live);
                    let transition = state
                        .lock()
                        .map(|mut s| {
                            if skip {
                                s.skipping.insert(job.id.clone())
                            } else {
                                s.terminals.remove(&job.id);
                                s.skipping.remove(&job.id)
                            }
                        })
                        .unwrap_or(false);
                    if skip {
                        if transition {
                            crate::app_instance::add_log(
                                &app,
                                &ctx.worktree_id,
                                &ctx.artifact_id,
                                "info",
                                format!(
                                    "スケジューラー: ジョブ '{}' が前回開いた AI ターミナルが残っているため、閉じられるまで実行を飛ばします",
                                    job.id
                                ),
                            );
                        }
                        continue;
                    }
                    if transition {
                        crate::app_instance::add_log(
                            &app,
                            &ctx.worktree_id,
                            &ctx.artifact_id,
                            "info",
                            format!("スケジューラー: ジョブ '{}' の AI ターミナルが閉じられたため、実行を再開します", job.id),
                        );
                    }
                }
            }
            let fresh = state.lock().map(|mut s| s.inflight.insert(job.id.clone())).unwrap_or(false);
            if !fresh {
                crate::app_instance::add_log(
                    &app,
                    &ctx.worktree_id,
                    &ctx.artifact_id,
                    "warn",
                    format!("スケジューラー: ジョブ '{}' は前回の実行が終わっていないため、この回を飛ばしました", job.id),
                );
                continue;
            }
            let app2 = app.clone();
            let label2 = label.clone();
            let state2 = state.clone();
            let resolved = grant.resolved.clone();
            tauri::async_runtime::spawn(async move {
                let opened = run_job(&app2, &label2, &resolved, &job).await;
                // 端末の記録を先に入れてから実行中の印を外す（同じロックの中で。隙間を作らない）
                if let Ok(mut s) = state2.lock() {
                    if let Some(tid) = opened {
                        s.terminals.insert(job.id.clone(), tid);
                    }
                    s.inflight.remove(&job.id);
                }
            });
        }
    }
}

async fn run_job(
    app: &AppHandle,
    label: &str,
    resolved: &crate::app_manifest::ResolvedPermissions,
    job: &SchedulerJob,
) -> Option<String> {
    // 実行の直前にもう一度、起動中かを確かめる（待っている間に停止されたかもしれない）
    let Ok(ctx) = crate::app_instance::context_for_label(app, label) else { return None };
    let log = |level: &str, msg: String| {
        crate::app_instance::add_log(app, &ctx.worktree_id, &ctx.artifact_id, level, msg);
    };
    log("info", format!("スケジューラー: ジョブ '{}' を実行します（{}）", job.id, job.cron));
    // 成功した aiTerminal ジョブが開いた terminal_id（次回を飛ばすかの判定に使う）
    let mut opened: Option<String> = None;
    let result: Result<String, String> = match &job.action {
        SchedulerAction::Tool { tool, params } => {
            crate::mcp_server::call_tool_for_app_instance(
                app,
                &ctx.worktree_id,
                &ctx.artifact_id,
                resolved,
                tool,
                params.clone(),
            )
            .await
            .map(|_| format!("tool {}", tool))
        }
        SchedulerAction::AiTerminal { title, initial_prompt, system_prompt, model, cwd } => {
            let app_name = crate::app_instance::instance_app_name(app, &ctx);
            let params = crate::ai_terminal::OpenParams {
                title: Some(title.clone().unwrap_or_else(|| format!("{} · {}", app_name, job.id))),
                system_prompt: system_prompt.clone(),
                initial_prompt: Some(initial_prompt.clone()),
                model: model.clone(),
                cwd: cwd.clone(),
            };
            crate::ai_terminal::open_inner(app, &ctx, params).await.map(|v| {
                opened = v.get("terminalId").and_then(|t| t.as_str()).map(str::to_string);
                "aiTerminal.open".to_string()
            })
        }
    };
    match result {
        Ok(what) => log("info", format!("許可: scheduler[{}] {}", job.id, what)),
        Err(e) => {
            let note = if ctx.reapproval_pending { "（再承認待ちのため、新しい権限はまだ使えません）" } else { "" };
            log("warn", format!("拒否: scheduler[{}] — {}{}", job.id, e, note));
        }
    }
    opened
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, 0).unwrap()
    }

    fn spec(s: &str) -> CronSpec {
        CronSpec::parse(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    #[test]
    fn normalizes_whitespace() {
        assert_eq!(normalize_cron("  0   9 *  * 1-5 "), "0 9 * * 1-5");
    }

    #[test]
    fn every_minute_matches_everything() {
        let s = spec("* * * * *");
        assert!(s.matches(&at(2026, 11, 3, 0, 0)));
        assert!(s.matches(&at(2026, 12, 31, 23, 59)));
    }

    #[test]
    fn fixed_time_and_lists_ranges_steps() {
        let s = spec("30 9 * * *");
        assert!(s.matches(&at(2026, 11, 3, 9, 30)));
        assert!(!s.matches(&at(2026, 11, 3, 9, 31)));
        assert!(!s.matches(&at(2026, 11, 3, 10, 30)));
        let s = spec("0,15,30,45 8-10 * * *");
        assert!(s.matches(&at(2026, 11, 3, 9, 15)));
        assert!(!s.matches(&at(2026, 11, 3, 11, 15)));
        let s = spec("*/20 * * * *");
        assert!(s.matches(&at(2026, 11, 3, 5, 0)) && s.matches(&at(2026, 11, 3, 5, 40)));
        assert!(!s.matches(&at(2026, 11, 3, 5, 10)));
        // a/n は a から上限まで
        let s = spec("10/25 * * * *");
        assert!(s.matches(&at(2026, 11, 3, 5, 10)) && s.matches(&at(2026, 11, 3, 5, 35)) && s.matches(&at(2026, 11, 3, 5, 35)));
        assert!(!s.matches(&at(2026, 11, 3, 5, 0)));
        // a-b/n
        let s = spec("0-30/10 * * * *");
        assert!(s.matches(&at(2026, 11, 3, 5, 20)) && !s.matches(&at(2026, 11, 3, 5, 40)));
    }

    #[test]
    fn weekday_sunday_is_zero_and_seven() {
        // 2026-11-01 は日曜、11-02 は月曜
        for expr in ["0 9 * * 0", "0 9 * * 7"] {
            let s = spec(expr);
            assert!(s.matches(&at(2026, 11, 1, 9, 0)), "{expr}");
            assert!(!s.matches(&at(2026, 11, 2, 9, 0)), "{expr}");
        }
        let weekdays = spec("0 9 * * 1-5");
        assert!(weekdays.matches(&at(2026, 11, 2, 9, 0)));
        assert!(!weekdays.matches(&at(2026, 11, 1, 9, 0)));
        // 土日
        let weekend = spec("0 9 * * 6,7");
        assert!(weekend.matches(&at(2026, 11, 1, 9, 0)) && weekend.matches(&at(2026, 10, 31, 9, 0)));
    }

    #[test]
    fn day_of_month_and_weekday_are_or_when_both_restricted() {
        // 毎月 15 日 または 月曜
        let s = spec("0 0 15 * 1");
        assert!(s.matches(&at(2026, 11, 15, 0, 0))); // 日曜の 15 日
        assert!(s.matches(&at(2026, 11, 2, 0, 0))); // 月曜（15 日ではない）
        assert!(!s.matches(&at(2026, 11, 3, 0, 0)));
        // 片方が * 始まりなら両方（AND）
        let s = spec("0 0 15 * *");
        assert!(s.matches(&at(2026, 11, 15, 0, 0)) && !s.matches(&at(2026, 11, 2, 0, 0)));
        let s = spec("0 0 */2 * 1");
        assert!(!s.matches(&at(2026, 11, 3, 0, 0))); // 火曜
        assert!(s.matches(&at(2026, 11, 9, 0, 0))); // 月曜かつ奇数日
    }

    #[test]
    fn month_field() {
        let s = spec("0 0 1 1,7 *");
        assert!(s.matches(&at(2026, 1, 1, 0, 0)) && s.matches(&at(2026, 7, 1, 0, 0)));
        assert!(!s.matches(&at(2026, 2, 1, 0, 0)));
    }

    #[test]
    fn rejects_bad_expressions() {
        for bad in [
            "", "* * * *", "* * * * * *", "60 * * * *", "* 24 * * *", "* * 0 * *", "* * 32 * *", "* * * 0 *",
            "* * * 13 *", "* * * * 8", "a * * * *", "*/0 * * * *", "5-1 * * * *", "1,,2 * * * *", "@daily", "*/x * * * *",
            "MON * * * *", "* * * JAN *",
        ] {
            assert!(CronSpec::parse(bad).is_err(), "{bad:?} は拒否されるべき");
        }
    }

    #[test]
    fn first_fire_window_is_exclusive_start_inclusive_end() {
        let s = spec("5 * * * *");
        // 10:05 ちょうどを after にすると、その回は含まない（二重発火しない）
        assert_eq!(s.first_fire_in(at(2026, 11, 3, 10, 5), at(2026, 11, 3, 10, 6)), None);
        assert_eq!(s.first_fire_in(at(2026, 11, 3, 10, 4), at(2026, 11, 3, 10, 5)), Some(at(2026, 11, 3, 10, 5)));
        // after の秒は切り捨てて分で比べる（10:04:59 → 10:05 は対象）
        let after = NaiveDate::from_ymd_opt(2026, 11, 3).unwrap().and_hms_opt(10, 4, 59).unwrap();
        assert_eq!(s.first_fire_in(after, at(2026, 11, 3, 10, 5)), Some(at(2026, 11, 3, 10, 5)));
        // 窓が逆（時計が戻った）なら発火しない
        assert_eq!(s.first_fire_in(at(2026, 11, 3, 10, 5), at(2026, 11, 3, 10, 0)), None);
    }

    #[test]
    fn skips_ai_job_only_while_its_previous_terminal_is_alive() {
        let live = vec!["t1".to_string(), "t2".to_string()];
        // 前回開いた端末が残っている → 飛ばす
        assert!(skip_ai_job(Some("t1"), &live));
        // 閉じられた・エージェントが終了した（生きている一覧に無い）→ 再開
        assert!(!skip_ai_job(Some("gone"), &live));
        assert!(!skip_ai_job(Some("t1"), &[]));
        // まだ一度も開いていない
        assert!(!skip_ai_job(None, &live));
    }

    #[test]
    fn sleeps_until_next_minute_boundary() {
        let t = NaiveDate::from_ymd_opt(2026, 11, 3).unwrap().and_hms_opt(10, 0, 20).unwrap();
        assert_eq!(sleep_until_next_minute(&t).as_secs(), 40);
        assert_eq!(sleep_until_next_minute(&at(2026, 11, 3, 10, 0)).as_secs(), 60);
    }
}
