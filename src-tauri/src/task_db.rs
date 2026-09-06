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

/// `list` の検索対象を steps JSON まで広げ、status での絞り込みを足した版。
///
/// UI の一覧は prompt で引ければ十分だが、MCP から「あのブランチのタスクはどうなったか」
/// を追うときは prompt を覚えていないことのほうが多く、リポジトリ名 / ブランチ名で
/// 引きたい。それらは `steps` JSON の中にしか無いので raw JSON ごと LIKE で舐める。
/// UI 側の検索挙動は変えたくないので別関数にしてある。
pub async fn list_filtered(
    pool: &SqlitePool,
    search: &str,
    status: &str,
    offset: i64,
    limit: i64,
) -> Result<TaskListResult, String> {
    let fetch_limit = limit + 1; // has_more 判定用に1件余分に取得
    let pattern = format!("%{}%", crate::escape_like(&search.to_ascii_lowercase()));
    let rows: Vec<TaskRow> = sqlx::query_as::<_, TaskRow>(
        "SELECT * FROM tasks \
         WHERE (?1 = '' OR LOWER(prompt) LIKE ?2 ESCAPE '\\' OR LOWER(steps) LIKE ?2 ESCAPE '\\') \
           AND (?3 = '' OR status = ?3) \
         ORDER BY created_at DESC LIMIT ?4 OFFSET ?5",
    )
    .bind(search)
    .bind(&pattern)
    .bind(status)
    .bind(fetch_limit)
    .bind(offset)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let has_more = rows.len() as i64 > limit;
    let mut rows = rows;
    if has_more {
        rows.truncate(limit as usize);
    }
    Ok(TaskListResult { items: rows, has_more })
}

/// タスクを1件取得する。存在しなければ `None`。
pub async fn get(pool: &SqlitePool, id: &str) -> Result<Option<TaskRow>, String> {
    sqlx::query_as::<_, TaskRow>("SELECT * FROM tasks WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())
}

/// 指定ワークツリー（repository + branch）宛のステップを含むタスクのプロンプトを新しい順に返す。
///
/// `tasks` テーブルはワークツリー ID を持たず、`steps` JSON の `code.repository` /
/// `code.branch` でしか紐付かない（`src/types/task.ts` の `TaskStep`）。SQL では表現しづらいので
/// 直近 `LIMIT` 件を取り出して Rust 側で突合する。
pub async fn list_prompts_for_worktree(
    pool: &SqlitePool,
    repository: &str,
    branch: &str,
    limit: i64,
) -> Result<Vec<String>, String> {
    let rows: Vec<TaskRow> = sqlx::query_as::<_, TaskRow>(
        "SELECT * FROM tasks ORDER BY created_at DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .into_iter()
        .filter(|row| steps_match_worktree(&row.steps, repository, branch))
        .map(|row| row.prompt)
        .collect())
}

/// `steps` JSON に該当ワークツリー宛のステップが含まれるか。
fn steps_match_worktree(steps_json: &str, repository: &str, branch: &str) -> bool {
    let Ok(steps) = serde_json::from_str::<serde_json::Value>(steps_json) else {
        return false;
    };
    let Some(arr) = steps.as_array() else {
        return false;
    };
    arr.iter().any(|step| {
        let code = step.get("code").unwrap_or(step);
        code.get("repository").and_then(|v| v.as_str()) == Some(repository)
            && code.get("branch").and_then(|v| v.as_str()) == Some(branch)
    })
}

pub async fn delete(pool: &SqlitePool, id: &str) -> Result<(), String> {
    sqlx::query("DELETE FROM tasks WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
