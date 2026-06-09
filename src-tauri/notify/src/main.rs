// oretachi worktree notification sidecar.
//
// Claude Code のライフサイクルフックから呼ばれ、起動中の oretachi MCP サーバへ
// HTTP POST で通知/プラン要約依頼を送る極小バイナリ。
// 旧来は GUI 本体 (oretachi.exe) を --notify / --set-description 付きで再起動して
// 同処理を行っていたが、数十MBの GUI バイナリ起動を避けるため独立サイドカーに分離した。
//
// 使い方:
//   oretachi-notify --notify "<worktree>" [--kind <kind>] [--agent <agent>]
//     (stdin がパイプの場合、その内容を body として /notify へ送信)
//   oretachi-notify --set-description "<worktree>"
//     (stdin の ExitPlanMode hook JSON を /set-description へ転送)

// Prevents a console window flash on Windows when spawned by hooks.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

const SERVER_INFO_FILE: &str = "mcp-server.json";

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // 通知 (--notify): stdin(hook JSON) を body として /notify へ送る
    if let Some(name) = find_notify_arg(&args) {
        let kind = find_kind_arg(&args);
        let agent = find_agent_arg(&args);
        let body = read_stdin_if_piped();
        if let Err(e) = send_notification(&name, kind.as_deref(), body.as_deref(), agent.as_deref()) {
            #[cfg(debug_assertions)]
            eprintln!("Notification failed: {}", e);
            let _ = e;
            std::process::exit(1);
        }
        std::process::exit(0);
    }

    // ExitPlanMode フック (--set-description): stdin の hook JSON を /set-description へ転送し、
    // 稼働中アプリにプランを AI 要約させてワークツリーの description にセットさせる。
    if let Some(name) = find_set_description_arg(&args) {
        let hook_json = read_stdin_if_piped();
        if let Err(e) = send_set_description(&name, hook_json.as_deref()) {
            #[cfg(debug_assertions)]
            eprintln!("Set description failed: {}", e);
            let _ = e;
            std::process::exit(1);
        }
        std::process::exit(0);
    }

    #[cfg(debug_assertions)]
    eprintln!("Usage: oretachi-notify --notify <worktree> [--kind <kind>] [--agent <agent>]\n       oretachi-notify --set-description <worktree>");
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

fn find_notify_arg(args: &[String]) -> Option<String> {
    find_arg(args, "--notify", "-n")
}

fn find_kind_arg(args: &[String]) -> Option<String> {
    find_arg(args, "--kind", "-k")
}

fn find_agent_arg(args: &[String]) -> Option<String> {
    find_arg(args, "--agent", "-a")
}

fn find_set_description_arg(args: &[String]) -> Option<String> {
    find_arg(args, "--set-description", "-d")
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
fn send_notification(
    worktree_name: &str,
    kind: Option<&str>,
    body: Option<&str>,
    agent: Option<&str>,
) -> Result<(), String> {
    let mut payload = serde_json::json!({
        "worktree": worktree_name,
        "kind": kind.unwrap_or("general"),
    });
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
fn send_set_description(worktree_name: &str, hook_json: Option<&str>) -> Result<(), String> {
    let mut payload = serde_json::json!({
        "worktree": worktree_name,
    });
    if let Some(j) = hook_json {
        payload["hookJson"] = serde_json::Value::String(j.to_string());
    }
    post_json("/set-description", &payload)
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
    fn test_find_notify_arg_long_space() {
        let args = vec!["bin".to_string(), "--notify".to_string(), "myname".to_string()];
        assert_eq!(find_notify_arg(&args), Some("myname".to_string()));
    }

    #[test]
    fn test_find_notify_arg_long_eq() {
        let args = vec!["bin".to_string(), "--notify=myname".to_string()];
        assert_eq!(find_notify_arg(&args), Some("myname".to_string()));
    }

    #[test]
    fn test_find_notify_arg_short() {
        let args = vec!["bin".to_string(), "-n".to_string(), "myname".to_string()];
        assert_eq!(find_notify_arg(&args), Some("myname".to_string()));
    }

    #[test]
    fn test_find_notify_arg_none() {
        let args = vec!["bin".to_string(), "--other".to_string()];
        assert_eq!(find_notify_arg(&args), None);
    }

    #[test]
    fn test_find_notify_arg_no_value() {
        let args = vec!["bin".to_string(), "--notify".to_string()];
        assert_eq!(find_notify_arg(&args), None);
    }

    #[test]
    fn test_find_notify_arg_empty() {
        let args: Vec<String> = vec!["bin".to_string()];
        assert_eq!(find_notify_arg(&args), None);
    }

    #[test]
    fn test_find_kind_arg_long_space() {
        let args = vec!["bin".to_string(), "--kind".to_string(), "approval".to_string()];
        assert_eq!(find_kind_arg(&args), Some("approval".to_string()));
    }

    #[test]
    fn test_find_kind_arg_long_eq() {
        let args = vec!["bin".to_string(), "--kind=completed".to_string()];
        assert_eq!(find_kind_arg(&args), Some("completed".to_string()));
    }

    #[test]
    fn test_find_kind_arg_short() {
        let args = vec!["bin".to_string(), "-k".to_string(), "general".to_string()];
        assert_eq!(find_kind_arg(&args), Some("general".to_string()));
    }

    #[test]
    fn test_find_kind_arg_none() {
        let args = vec!["bin".to_string(), "--notify".to_string(), "myname".to_string()];
        assert_eq!(find_kind_arg(&args), None);
    }

    #[test]
    fn test_find_agent_arg_long_space() {
        let args = vec!["bin".to_string(), "--agent".to_string(), "cc".to_string()];
        assert_eq!(find_agent_arg(&args), Some("cc".to_string()));
    }

    #[test]
    fn test_find_agent_arg_long_eq() {
        let args = vec!["bin".to_string(), "--agent=codex".to_string()];
        assert_eq!(find_agent_arg(&args), Some("codex".to_string()));
    }

    #[test]
    fn test_find_agent_arg_short() {
        let args = vec!["bin".to_string(), "-a".to_string(), "gemini".to_string()];
        assert_eq!(find_agent_arg(&args), Some("gemini".to_string()));
    }

    #[test]
    fn test_find_agent_arg_none() {
        let args = vec!["bin".to_string(), "--notify".to_string(), "myname".to_string()];
        assert_eq!(find_agent_arg(&args), None);
    }

    #[test]
    fn test_find_set_description_arg_long_space() {
        let args = vec!["bin".to_string(), "--set-description".to_string(), "wt".to_string()];
        assert_eq!(find_set_description_arg(&args), Some("wt".to_string()));
    }

    #[test]
    fn test_find_set_description_arg_short() {
        let args = vec!["bin".to_string(), "-d".to_string(), "wt".to_string()];
        assert_eq!(find_set_description_arg(&args), Some("wt".to_string()));
    }

    #[test]
    fn test_find_set_description_arg_none() {
        let args = vec!["bin".to_string(), "--notify".to_string(), "wt".to_string()];
        assert_eq!(find_set_description_arg(&args), None);
    }
}
