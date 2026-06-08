use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::Manager;

/// `manage()` で複数の SqlitePool を区別するためのnewtypeラッパー
pub struct WorktreeDescPool(pub SqlitePool);

/// ワークツリーの description の正本（settings.json ではなく DB を正とする）
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow, Clone)]
pub struct WorktreeDescRow {
    pub worktree_id: String,
    pub description: String,
    pub updated_at: i64,
}

pub async fn init_worktree_desc_db(
    app: &tauri::AppHandle,
) -> Result<SqlitePool, Box<dyn std::error::Error>> {
    let app_data_dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&app_data_dir)?;
    let db_path = app_data_dir.join("worktree_descriptions.db");
    let db_url = format!("sqlite:{}?mode=rwc", db_path.display());
    let pool = SqlitePool::connect(&db_url).await?;
    run_migrations(&pool).await?;
    Ok(pool)
}

async fn run_migrations(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS worktree_descriptions (
            worktree_id TEXT PRIMARY KEY,
            description TEXT NOT NULL,
            updated_at  INTEGER NOT NULL
        )"#,
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn upsert(
    pool: &SqlitePool,
    worktree_id: &str,
    description: &str,
    updated_at: i64,
) -> Result<(), String> {
    sqlx::query(
        "INSERT OR REPLACE INTO worktree_descriptions (worktree_id, description, updated_at) VALUES (?, ?, ?)",
    )
    .bind(worktree_id)
    .bind(description)
    .bind(updated_at)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn get_all(pool: &SqlitePool) -> Result<Vec<WorktreeDescRow>, String> {
    sqlx::query_as::<_, WorktreeDescRow>("SELECT * FROM worktree_descriptions")
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())
}

pub async fn delete(pool: &SqlitePool, worktree_id: &str) -> Result<(), String> {
    sqlx::query("DELETE FROM worktree_descriptions WHERE worktree_id = ?")
        .bind(worktree_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 旧 settings.json に保存されていた description を DB へ一度だけ移行する。
/// DB に未登録の worktree_id のみ取り込むため、繰り返し呼ばれても安全。
pub async fn migrate_from_settings_file(pool: &SqlitePool, settings_path: &std::path::Path) {
    let Ok(content) = std::fs::read_to_string(settings_path) else {
        return;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
        return;
    };
    let Some(worktrees) = json.get("worktrees").and_then(|v| v.as_array()) else {
        return;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    for wt in worktrees {
        let (Some(id), Some(desc)) = (
            wt.get("id").and_then(|v| v.as_str()),
            wt.get("description").and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        if desc.trim().is_empty() {
            continue;
        }
        let exists: Option<(String,)> =
            sqlx::query_as("SELECT worktree_id FROM worktree_descriptions WHERE worktree_id = ?")
                .bind(id)
                .fetch_optional(pool)
                .await
                .ok()
                .flatten();
        if exists.is_none() {
            let _ = upsert(pool, id, desc.trim(), now).await;
        }
    }
}
