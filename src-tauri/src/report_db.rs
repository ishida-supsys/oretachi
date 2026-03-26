use serde::Serialize;
use sqlx::SqlitePool;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::Manager;

/// `manage()` で複数の SqlitePool を区別するためのnewtypeラッパー
pub struct ReportPool(pub SqlitePool);

#[derive(Debug, Serialize)]
pub struct ReportSummary {
    pub worktree_added: i64,
    pub worktree_removed: i64,
    pub artifact_added: i64,
    pub artifact_removed: i64,
    pub ai_result_count: i64,
}

/// UTC の ISO 8601 文字列を chrono を使わずに生成する
fn now_iso8601() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    let days = secs / 86400;
    // 2000-01-01 = UNIX epoch day 10957
    let days_since_2000 = days as i64 - 10957;
    let (y, mo, d) = days_to_ymd(days_since_2000);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, d, h, m, s)
}

fn days_to_ymd(mut days: i64) -> (i64, i64, i64) {
    let mut y = 2000i64;
    loop {
        let leap = is_leap(y);
        let dy = if leap { 366 } else { 365 };
        if days < dy { break; }
        days -= dy;
        y += 1;
    }
    let month_days: [i64; 12] = [31, if is_leap(y) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut mo = 1i64;
    for md in &month_days {
        if days < *md { break; }
        days -= md;
        mo += 1;
    }
    (y, mo, days + 1)
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

pub async fn init_report_db(
    app: &tauri::AppHandle,
) -> Result<SqlitePool, Box<dyn std::error::Error>> {
    let app_data_dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&app_data_dir)?;
    let db_path = app_data_dir.join("reports.db");
    let db_url = format!("sqlite:{}?mode=rwc", db_path.display());
    let pool = SqlitePool::connect(&db_url).await?;
    run_migrations(&pool).await?;
    Ok(pool)
}

async fn run_migrations(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS reports (
            id       INTEGER PRIMARY KEY,
            kind     TEXT NOT NULL,
            content  TEXT NOT NULL DEFAULT '',
            event_at TEXT NOT NULL
        )"#,
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_reports_kind_event ON reports(kind, event_at)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn insert(pool: &SqlitePool, kind: &str, content: &str) -> Result<(), String> {
    let now = now_iso8601();
    sqlx::query("INSERT INTO reports (kind, content, event_at) VALUES (?, ?, ?)")
        .bind(kind)
        .bind(content)
        .bind(&now)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// ローカル日付 "YYYY-MM-DD" とタイムゾーンオフセット（分、西向き正）から
/// UTC の開始・終了 ISO8601 文字列を計算する
fn local_date_to_utc_range(date: &str, tz_offset_min: i64) -> Option<(String, String)> {
    let parts: Vec<&str> = date.split('-').collect();
    if parts.len() != 3 { return None; }
    let (y, mo, d): (i64, i64, i64) = (
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
    );
    // ローカル 00:00:00 の UNIX秒
    let local_start_secs = ymd_to_unix(y, mo, d)? + (tz_offset_min * 60);
    let local_end_secs = local_start_secs + 86400;
    Some((secs_to_iso8601(local_start_secs), secs_to_iso8601(local_end_secs)))
}

fn ymd_to_unix(y: i64, mo: i64, d: i64) -> Option<i64> {
    if mo < 1 || mo > 12 || d < 1 || d > 31 { return None; }
    // 1970-01-01 からの日数
    let mut days: i64 = 0;
    for yr in 1970..y {
        days += if is_leap(yr) { 366 } else { 365 };
    }
    let month_days: [i64; 12] = [31, if is_leap(y) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for m in 0..(mo as usize - 1) {
        days += month_days[m];
    }
    days += d - 1;
    Some(days * 86400)
}

fn secs_to_iso8601(secs: i64) -> String {
    let secs = secs.max(0) as u64;
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    let days = secs / 86400;
    let days_since_2000 = days as i64 - 10957;
    let (y, mo, d) = days_to_ymd(days_since_2000);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, d, h, m, s)
}

async fn count_records(
    pool: &SqlitePool,
    kind_prefix: &str,
    utc_start: &str,
    utc_end: &str,
) -> Result<i64, String> {
    let pattern = format!("{}%", kind_prefix);
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM reports WHERE kind LIKE ? AND event_at >= ? AND event_at < ?",
    )
    .bind(&pattern)
    .bind(utc_start)
    .bind(utc_end)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())
}

pub async fn summary_for_date(
    pool: &SqlitePool,
    date: &str,
    tz_offset_min: i64,
) -> Result<ReportSummary, String> {
    let (utc_start, utc_end) = local_date_to_utc_range(date, tz_offset_min)
        .ok_or_else(|| format!("Invalid date: {}", date))?;
    Ok(ReportSummary {
        worktree_added:   count_records(pool, "worktree_change:add",    &utc_start, &utc_end).await?,
        worktree_removed: count_records(pool, "worktree_change:remove", &utc_start, &utc_end).await?,
        artifact_added:   count_records(pool, "artifact_change:create", &utc_start, &utc_end).await?,
        artifact_removed: count_records(pool, "artifact_change:delete", &utc_start, &utc_end).await?,
        ai_result_count:  count_records(pool, "ai_result:",             &utc_start, &utc_end).await?,
    })
}
