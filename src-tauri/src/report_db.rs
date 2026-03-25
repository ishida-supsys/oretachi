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

async fn count_records(pool: &SqlitePool, kind_prefix: &str, date: &str) -> Result<i64, String> {
    let pattern = format!("{}%", kind_prefix);
    let like = format!("{}%", date);
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM reports WHERE kind LIKE ? AND event_at LIKE ?",
    )
    .bind(&pattern)
    .bind(&like)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())
}

pub async fn summary_for_date(pool: &SqlitePool, date: &str) -> Result<ReportSummary, String> {
    Ok(ReportSummary {
        worktree_added: count_records(pool, "worktree_change:add", date).await?,
        worktree_removed: count_records(pool, "worktree_change:remove", date).await?,
        artifact_added: count_records(pool, "artifact_change:create", date).await?,
        artifact_removed: count_records(pool, "artifact_change:delete", date).await?,
        ai_result_count: count_records(pool, "ai_result:", date).await?,
    })
}
