// oretachi worktree notification sidecar.
//
// Claude Code のライフサイクルフックから呼ばれ、起動中の oretachi MCP サーバへ
// HTTP POST で通知/プラン要約依頼を送る極小バイナリ。
// 旧来は GUI 本体 (oretachi.exe) を --notify / --set-description 付きで再起動して
// 同処理を行っていたが、数十MBの GUI バイナリ起動を避けるため独立サイドカーに分離した。
//
// 使い方:
//   oretachi-notify --notify --project-dir "<dir>" --event "<Event>" [--agent <agent>]
//     (stdin がパイプの場合、その内容を body として /notify へ送信。
//      ワークツリー名と kind の解決はサーバー側で project-dir / event から行う)
//   oretachi-notify --set-description --project-dir "<dir>"
//     (stdin の ExitPlanMode hook JSON を /set-description へ転送)
//   oretachi-notify --prompt-context --project-dir "<dir>"
//     (UserPromptSubmit フック用。/prompt-context から現在の description を取得し、
//      additionalContext JSON を stdout に出力して Claude のコンテキストに注入する)
//
// hook からは userConfig ではなく CC 組み込み変数 ${CLAUDE_PROJECT_DIR} が --project-dir に渡る。
// 未置換/空の場合はプロセスの current_dir (hook は worktree ディレクトリで実行される) にフォールバック。

// Prevents a console window flash on Windows when spawned by hooks.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

const SERVER_INFO_FILE: &str = "mcp-server.json";

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // ExitPlanMode フック (--set-description): stdin の hook JSON を /set-description へ転送し、
    // 稼働中アプリにプランを AI 要約させてワークツリーの description にセットさせる。
    // --notify より先に判定する（両フラグが同時指定されることは無いが順序を明示）。
    if has_flag(&args, "--set-description", "-d") {
        let dir = resolve_project_dir(&args);
        let hook_json = read_stdin_if_piped();
        if let Err(e) = send_set_description(&dir, hook_json.as_deref()) {
            #[cfg(debug_assertions)]
            eprintln!("Set description failed: {}", e);
            let _ = e;
            std::process::exit(1);
        }
        std::process::exit(0);
    }

    // UserPromptSubmit フック (--prompt-context): /prompt-context から現在の description を
    // 取得し、additionalContext JSON を stdout に出力する。スロットル中 (skip) やサーバ
    // 未起動時は何も出力しない。フックをブロックしないよう常に exit 0 で終える。
    if has_flag(&args, "--prompt-context", "-c") {
        let dir = resolve_project_dir(&args);
        match send_prompt_context(&dir) {
            Ok(Some(output)) => println!("{}", output),
            Ok(None) => {}
            Err(_e) => {
                #[cfg(debug_assertions)]
                eprintln!("Prompt context failed: {}", _e);
            }
        }
        std::process::exit(0);
    }

    // 通知 (--notify): stdin(hook JSON) を body として /notify へ送る。
    // ワークツリー名と kind はサーバー側で project-dir / event から解決する。
    if has_flag(&args, "--notify", "-n") {
        let dir = resolve_project_dir(&args);
        let event = find_event_arg(&args);
        let agent = find_agent_arg(&args);
        let body = read_stdin_if_piped();
        if let Err(e) = send_notification(&dir, event.as_deref(), body.as_deref(), agent.as_deref()) {
            #[cfg(debug_assertions)]
            eprintln!("Notification failed: {}", e);
            let _ = e;
            std::process::exit(1);
        }
        std::process::exit(0);
    }

    #[cfg(debug_assertions)]
    eprintln!("Usage: oretachi-notify --notify --project-dir <dir> --event <Event> [--agent <agent>]\n       oretachi-notify --set-description --project-dir <dir>\n       oretachi-notify --prompt-context --project-dir <dir>");
    std::process::exit(2);
}

fn find_arg(args: &[String], long: &str, short: &str) -> Option<String> {
    let long_eq = format!("{}=", long);
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        if arg == long || arg == short {
            return iter.next().cloned();
        } else if arg.starts_with(&long_eq) {
            return Some(arg[long_eq.len()..].to_string());
        }
    }
    None
}

/// 値を取らないフラグ（--notify / --set-description）の有無を判定する。
fn has_flag(args: &[String], long: &str, short: &str) -> bool {
    args.iter().skip(1).any(|a| a == long || a == short)
}

fn find_project_dir_arg(args: &[String]) -> Option<String> {
    find_arg(args, "--project-dir", "-p")
}

fn find_event_arg(args: &[String]) -> Option<String> {
    find_arg(args, "--event", "-e")
}

fn find_agent_arg(args: &[String]) -> Option<String> {
    find_arg(args, "--agent", "-a")
}

/// --project-dir を解決する。${CLAUDE_PROJECT_DIR} が未置換のまま届いた場合や空/未指定の
/// 場合は、hook が実行される worktree ディレクトリ = プロセスの current_dir にフォールバック。
fn resolve_project_dir(args: &[String]) -> String {
    let raw = find_project_dir_arg(args);
    match raw {
        Some(d) if !d.is_empty() && !d.contains("${") => d,
        _ => std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
    }
}

/// stdin がパイプ（非 TTY）の場合のみ読み取り、タイムアウト付きで返す。
/// Claude Code ライフサイクルフックのコンテキスト JSON を body として受け取るために使用。
fn read_stdin_if_piped() -> Option<String> {
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        return None;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = String::new();
        let _ = std::io::stdin().read_to_string(&mut buf);
        let _ = tx.send(buf);
    });
    match rx.recv_timeout(std::time::Duration::from_secs(2)) {
        Ok(s) if !s.is_empty() => Some(s),
        _ => None,
    }
}

/// 起動中の oretachi MCP サーバへ通知を送る（AppHandle 不要のスタンドアロン実装）。
/// ワークツリー名・kind の解決はサーバー側で project_dir / event から行うため、ここでは
/// 生の project_dir と event を渡す。
fn send_notification(
    project_dir: &str,
    event: Option<&str>,
    body: Option<&str>,
    agent: Option<&str>,
) -> Result<(), String> {
    let mut payload = serde_json::json!({
        "projectDir": project_dir,
    });
    if let Some(e) = event {
        payload["event"] = serde_json::Value::String(e.to_string());
    }
    if let Some(b) = body {
        payload["body"] = serde_json::Value::String(b.to_string());
    }
    if let Some(a) = agent {
        payload["agent"] = serde_json::Value::String(a.to_string());
    }
    post_json("/notify", &payload)
}

/// ExitPlanMode フックの hook JSON を /set-description へ転送し、
/// 稼働中アプリにプランの AI 要約と description セットを依頼する。
/// ワークツリーの特定はサーバー側で project_dir から行う。
fn send_set_description(project_dir: &str, hook_json: Option<&str>) -> Result<(), String> {
    let mut payload = serde_json::json!({
        "projectDir": project_dir,
    });
    if let Some(j) = hook_json {
        payload["hookJson"] = serde_json::Value::String(j.to_string());
    }
    post_json("/set-description", &payload)
}

/// UserPromptSubmit フック用。/prompt-context から現在の description を取得し、
/// stdout に出力すべき additionalContext JSON を返す。skip 時は Ok(None)。
fn send_prompt_context(project_dir: &str) -> Result<Option<String>, String> {
    let payload = serde_json::json!({
        "projectDir": project_dir,
    });
    let body = post_json_read_body("/prompt-context", &payload)?;
    Ok(build_prompt_context_output(&body))
}

/// /prompt-context のレスポンスボディから UserPromptSubmit 用の
/// additionalContext JSON を組み立てる。skip / parse 失敗時は None。
fn build_prompt_context_output(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    if v["skip"].as_bool() == Some(true) {
        return None;
    }
    let context = match v["description"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
        Some(desc) => format!(
            "[oretachi] このワークツリーの現在の description: 「{}」。これは作業全体の目的を表す1行です。今の作業がこの説明の範囲内（同一プランのサブタスク進行・レビュー対応など）なら更新は不要です。全く別の作業に切り替わった場合、または説明が実態と大きくずれている場合のみ oretachi_set_description ツールで更新してください。",
            desc
        ),
        None => "[oretachi] このワークツリーの description は未設定です。作業内容が決まっていれば oretachi_set_description ツールで作業全体の目的を1行でセットしてください。".to_string(),
    };
    let output = serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "UserPromptSubmit",
            "additionalContext": context,
        }
    });
    Some(output.to_string())
}

/// post_json と同様に POST し、レスポンスボディまで読んで返す。
/// /prompt-context のようにサーバの返す JSON が必要な場合に使う。
fn post_json_read_body(path: &str, payload: &serde_json::Value) -> Result<String, String> {
    let (port, api_key) = read_server_info()?;
    let payload_str = payload.to_string();
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nAuthorization: Bearer {api_key}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload_str}",
        payload_str.len()
    );

    use std::io::{Read, Write};
    use std::time::Duration;

    let addr: std::net::SocketAddr = format!("127.0.0.1:{}", port)
        .parse()
        .map_err(|e| format!("Invalid address: {}", e))?;
    let mut stream = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(3))
        .map_err(|e| format!("Cannot connect to oretachi MCP server: {}", e))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| format!("Failed to set write timeout: {}", e))?;
    // body まで必要なので post_json より長めの読み取りタイムアウト（ローカル接続なので通常は数ms）
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| format!("Failed to set read timeout: {}", e))?;
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("Failed to send request: {}", e))?;
    stream
        .flush()
        .map_err(|e| format!("Failed to flush: {}", e))?;

    // Connection: close なのでサーバが閉じるまで読み切る（タイムアウト時は読めた分まで）
    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => raw.extend_from_slice(&buf[..n]),
            Err(_) => break,
        }
    }
    let response = String::from_utf8_lossy(&raw);
    let (head, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| "Incomplete HTTP response".to_string())?;
    let status_line = head.lines().next().unwrap_or_default();
    if !status_line.contains(" 200 ") {
        return Err(format!("Server returned unexpected response: {}", status_line));
    }
    // axum の Json レスポンスは Content-Length 付き（chunked ではない）前提でそのまま返す
    Ok(body.to_string())
}

/// mcp-server.json のポート/APIキーを読み、指定パスへ JSON を POST する。
/// フックをブロックしないようタイムアウトは短く、応答読み取りはベストエフォート。
fn post_json(path: &str, payload: &serde_json::Value) -> Result<(), String> {
    let (port, api_key) = read_server_info()?;
    let payload_str = payload.to_string();
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nAuthorization: Bearer {api_key}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload_str}",
        payload_str.len()
    );

    use std::io::Write;
    use std::time::Duration;

    let addr: std::net::SocketAddr = format!("127.0.0.1:{}", port)
        .parse()
        .map_err(|e| format!("Invalid address: {}", e))?;
    let mut stream = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(3))
        .map_err(|e| format!("Cannot connect to oretachi MCP server: {}", e))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| format!("Failed to set write timeout: {}", e))?;
    // 短い読み取りタイムアウトで応答をチェック（非ブロッキング性を維持しつつ確実な配信失敗を検出）
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .map_err(|e| format!("Failed to set read timeout: {}", e))?;
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("Failed to send request: {}", e))?;
    stream
        .flush()
        .map_err(|e| format!("Failed to flush: {}", e))?;

    // 応答の最初の行だけ読んでステータスを確認する（タイムアウトは無視してベストエフォートとする）
    use std::io::{BufRead, BufReader};
    let reader = BufReader::new(&stream);
    if let Some(Ok(first_line)) = reader.lines().next() {
        if first_line.starts_with("HTTP/") && !first_line.contains(" 200 ") {
            return Err(format!("Server returned unexpected response: {}", first_line));
        }
    }
    // タイムアウトや読み取りエラーはベストエフォート成功扱い（フックのブロック防止）
    Ok(())
}

/// mcp-server.json から MCP サーバのポートと API キーを読み取る。
/// 保存場所: Windows は %APPDATA%/com.ia.oretachi、その他は ~/Library/Application Support/com.ia.oretachi。
fn read_server_info() -> Result<(u16, String), String> {
    #[cfg(target_os = "windows")]
    let base = {
        let appdata = std::env::var("APPDATA")
            .map_err(|_| "APPDATA environment variable not set".to_string())?;
        PathBuf::from(appdata).join("com.ia.oretachi")
    };

    #[cfg(not(target_os = "windows"))]
    let base = {
        let home = std::env::var("HOME")
            .map_err(|_| "HOME environment variable not set".to_string())?;
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("com.ia.oretachi")
    };

    let json_path = base.join(SERVER_INFO_FILE);
    if json_path.exists() {
        let content = std::fs::read_to_string(&json_path)
            .map_err(|e| format!("Cannot read server info file: {}", e))?;
        let info: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| format!("Invalid server info JSON: {}", e))?;
        let port = info["port"]
            .as_u64()
            .ok_or_else(|| "Missing port in server info".to_string())? as u16;
        let api_key = info["apiKey"]
            .as_str()
            .ok_or_else(|| "Missing apiKey in server info".to_string())?
            .to_string();
        return Ok((port, api_key));
    }

    Err("Cannot read API key: mcp-server.json not found. Please restart oretachi to regenerate the server info file.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_has_flag_notify() {
        let args = vec!["bin".to_string(), "--notify".to_string(), "--agent".to_string(), "cc".to_string()];
        assert!(has_flag(&args, "--notify", "-n"));
        assert!(!has_flag(&args, "--set-description", "-d"));
    }

    #[test]
    fn test_has_flag_short() {
        let args = vec!["bin".to_string(), "-d".to_string()];
        assert!(has_flag(&args, "--set-description", "-d"));
    }

    #[test]
    fn test_has_flag_absent() {
        let args = vec!["bin".to_string(), "--other".to_string()];
        assert!(!has_flag(&args, "--notify", "-n"));
    }

    #[test]
    fn test_has_flag_prompt_context() {
        let args = vec!["bin".to_string(), "--prompt-context".to_string(), "--project-dir".to_string(), "X:/wt/foo".to_string()];
        assert!(has_flag(&args, "--prompt-context", "-c"));
        assert!(!has_flag(&args, "--notify", "-n"));
    }

    #[test]
    fn test_build_prompt_context_output_with_description() {
        let body = r#"{"worktreeName":"foo","description":"認証機能のリファクタリング"}"#;
        let out = build_prompt_context_output(body).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["hookSpecificOutput"]["hookEventName"], "UserPromptSubmit");
        let ctx = v["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
        assert!(ctx.contains("認証機能のリファクタリング"));
        assert!(ctx.contains("oretachi_set_description"));
    }

    #[test]
    fn test_build_prompt_context_output_without_description() {
        let body = r#"{"worktreeName":"foo","description":null}"#;
        let out = build_prompt_context_output(body).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let ctx = v["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
        assert!(ctx.contains("未設定"));
        assert!(ctx.contains("oretachi_set_description"));
    }

    #[test]
    fn test_build_prompt_context_output_skip() {
        assert_eq!(build_prompt_context_output(r#"{"skip":true}"#), None);
    }

    #[test]
    fn test_build_prompt_context_output_invalid_json() {
        assert_eq!(build_prompt_context_output("not json"), None);
    }

    #[test]
    fn test_find_project_dir_long_space() {
        let args = vec!["bin".to_string(), "--project-dir".to_string(), "X:/wt/foo".to_string()];
        assert_eq!(find_project_dir_arg(&args), Some("X:/wt/foo".to_string()));
    }

    #[test]
    fn test_find_project_dir_long_eq() {
        let args = vec!["bin".to_string(), "--project-dir=X:/wt/foo".to_string()];
        assert_eq!(find_project_dir_arg(&args), Some("X:/wt/foo".to_string()));
    }

    #[test]
    fn test_find_project_dir_short() {
        let args = vec!["bin".to_string(), "-p".to_string(), "X:/wt/foo".to_string()];
        assert_eq!(find_project_dir_arg(&args), Some("X:/wt/foo".to_string()));
    }

    #[test]
    fn test_resolve_project_dir_explicit() {
        let args = vec!["bin".to_string(), "--project-dir".to_string(), "X:/wt/foo".to_string()];
        assert_eq!(resolve_project_dir(&args), "X:/wt/foo".to_string());
    }

    #[test]
    fn test_resolve_project_dir_unsubstituted_falls_back_to_cwd() {
        // ${CLAUDE_PROJECT_DIR} が未置換のまま届いたら current_dir にフォールバックする
        let args = vec!["bin".to_string(), "--project-dir".to_string(), "${CLAUDE_PROJECT_DIR}".to_string()];
        let cwd = std::env::current_dir().unwrap().to_string_lossy().to_string();
        assert_eq!(resolve_project_dir(&args), cwd);
    }

    #[test]
    fn test_resolve_project_dir_missing_falls_back_to_cwd() {
        let args = vec!["bin".to_string(), "--notify".to_string()];
        let cwd = std::env::current_dir().unwrap().to_string_lossy().to_string();
        assert_eq!(resolve_project_dir(&args), cwd);
    }

    #[test]
    fn test_find_event_arg() {
        let args = vec!["bin".to_string(), "--event".to_string(), "Stop".to_string()];
        assert_eq!(find_event_arg(&args), Some("Stop".to_string()));
        let none = vec!["bin".to_string(), "--notify".to_string()];
        assert_eq!(find_event_arg(&none), None);
    }

    #[test]
    fn test_find_agent_arg_long_space() {
        let args = vec!["bin".to_string(), "--agent".to_string(), "cc".to_string()];
        assert_eq!(find_agent_arg(&args), Some("cc".to_string()));
    }

    #[test]
    fn test_find_agent_arg_short() {
        let args = vec!["bin".to_string(), "-a".to_string(), "gemini".to_string()];
        assert_eq!(find_agent_arg(&args), Some("gemini".to_string()));
    }

    #[test]
    fn test_find_agent_arg_none() {
        let args = vec!["bin".to_string(), "--notify".to_string(), "--event".to_string(), "Stop".to_string()];
        assert_eq!(find_agent_arg(&args), None);
    }
}
