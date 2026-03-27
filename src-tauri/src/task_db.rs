use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::Manager;

/// `manage()` で複数の SqlitePool を区別するためのnewtypeラッパー
pub struct TaskPool(pub SqlitePool);

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow, Clone)]
pub struct TaskRow {
    pub id: String,
    pub prompt: String,
    pub created_at: i64,
    pub status: String,
    pub steps: String,       // JSON string: TaskStep[]
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TaskListResult {
    pub items: Vec<TaskRow>,
    pub has_more: bool,
}

pub async fn init_task_db(
    app: &tauri::AppHandle,
) -> Result<SqlitePool, Box<dyn std::error::Error>> {
    let app_data_dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&app_data_dir)?;
    let db_path = app_data_dir.join("tasks.db");
    let db_url = format!("sqlite:{}?mode=rwc", db_path.display());
    let pool = SqlitePool::connect(&db_url).await?;
    run_migrations(&pool).await?;
    Ok(pool)
}

async fn run_migrations(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS tasks (
            id         TEXT PRIMARY KEY,
            prompt     TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            status     TEXT NOT NULL,
            steps      TEXT NOT NULL DEFAULT '[]',
            error      TEXT
        )"#,
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_tasks_created_at ON tasks(created_at DESC)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_tasks_prompt ON tasks(prompt)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn save(pool: &SqlitePool, task: &TaskRow) -> Result<(), String> {
    sqlx::query(
        "INSERT OR REPLACE INTO tasks (id, prompt, created_at, status, steps, error) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&task.id)
    .bind(&task.prompt)
    .bind(task.created_at)
    .bind(&task.status)
    .bind(&task.steps)
    .bind(&task.error)
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
) -> Result<TaskListResult, String> {
    let fetch_limit = limit + 1; // has_more 判定用に1件余分に取得
    let mut rows: Vec<TaskRow> = if search.is_empty() {
        sqlx::query_as::<_, TaskRow>(
            "SELECT * FROM tasks ORDER BY created_at DESC LIMIT ? OFFSET ?",
        )
        .bind(fetch_limit)
        .bind(offset)
        .fetch_all(pool)
        .await
    } else {
        let pattern = format!("%{}%", search);
        sqlx::query_as::<_, TaskRow>(
            "SELECT * FROM tasks WHERE prompt LIKE ? ORDER BY created_at DESC LIMIT ? OFFSET ?",
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
    Ok(TaskListResult { items: rows, has_more })
}

pub async fn delete(pool: &SqlitePool, id: &str) -> Result<(), String> {
    sqlx::query("DELETE FROM tasks WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
