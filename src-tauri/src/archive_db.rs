use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::Manager;

/// `manage()` で複数の SqlitePool を区別するためのnewtypeラッパー
pub struct ArchivePool(pub SqlitePool);

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow, Clone)]
pub struct ArchiveRow {
    pub id: String,
    pub name: String,
    pub repository_id: String,
    pub repository_name: String,
    pub path: String,
    pub branch_name: String,
    pub archived_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ArchiveListResult {
    pub items: Vec<ArchiveRow>,
    pub has_more: bool,
}

pub async fn init_archive_db(
    app: &tauri::AppHandle,
) -> Result<SqlitePool, Box<dyn std::error::Error>> {
    let app_data_dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&app_data_dir)?;
    let db_path = app_data_dir.join("archives.db");
    let db_url = format!("sqlite:{}?mode=rwc", db_path.display());
    let pool = SqlitePool::connect(&db_url).await?;
    run_migrations(&pool).await?;
    Ok(pool)
}

async fn run_migrations(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS archives (
            id              TEXT PRIMARY KEY,
            name            TEXT NOT NULL,
            repository_id   TEXT NOT NULL,
            repository_name TEXT NOT NULL,
            path            TEXT NOT NULL,
            branch_name     TEXT NOT NULL,
            archived_at     INTEGER NOT NULL
        )"#,
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_archives_archived_at ON archives(archived_at DESC)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_archives_name ON archives(name)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn save(pool: &SqlitePool, archive: &ArchiveRow) -> Result<(), String> {
    sqlx::query(
        "INSERT OR REPLACE INTO archives (id, name, repository_id, repository_name, path, branch_name, archived_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&archive.id)
    .bind(&archive.name)
    .bind(&archive.repository_id)
    .bind(&archive.repository_name)
    .bind(&archive.path)
    .bind(&archive.branch_name)
    .bind(archive.archived_at)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn list(
    pool: &SqlitePool,
    search: &str,
    offset: i64,
    limit: i64,
) -> Result<ArchiveListResult, String> {
    let fetch_limit = limit + 1; // has_more 判定用に1件余分に取得
    let mut rows: Vec<ArchiveRow> = if search.is_empty() {
        sqlx::query_as::<_, ArchiveRow>(
            "SELECT * FROM archives ORDER BY archived_at DESC LIMIT ? OFFSET ?",
        )
        .bind(fetch_limit)
        .bind(offset)
        .fetch_all(pool)
        .await
    } else {
        let pattern = format!("%{}%", search);
        sqlx::query_as::<_, ArchiveRow>(
            "SELECT * FROM archives WHERE name LIKE ? ORDER BY archived_at DESC LIMIT ? OFFSET ?",
        )
        .bind(&pattern)
        .bind(fetch_limit)
        .bind(offset)
        .fetch_all(pool)
        .await
    }
    .map_err(|e| e.to_string())?;

    let has_more = rows.len() as i64 > limit;
    if has_more {
        rows.truncate(limit as usize);
    }
    Ok(ArchiveListResult { items: rows, has_more })
}

pub async fn delete(pool: &SqlitePool, id: &str) -> Result<(), String> {
    sqlx::query("DELETE FROM archives WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
