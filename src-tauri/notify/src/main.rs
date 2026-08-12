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
//   oretachi-notify --session-context --project-dir "<dir>"
//     (SessionStart フック用。/session-context からワークツリー所属グループの systemPrompt を
//      取得し、SessionStart 用 JSON として stdout に出力する。失敗時は何も出力せず exit 0)
//   oretachi-notify --prompt-context --project-dir "<dir>"
//     (UserPromptSubmit フック用。/prompt-context から現在の description と未読を取得し、
//      additionalContext JSON を stdout に出力して Claude のコンテキストに注入する)
//   oretachi-notify --turn-context --project-dir "<dir>"
//     (Stop フック用。stdin の Stop hook JSON を /turn-context へ転送し、このタブ宛の未読を
//      additionalContext として stdout に出力する。会話が継続してエージェントが着手する)
//
// hook からは userConfig ではなく CC 組み込み変数 ${CLAUDE_PROJECT_DIR} が --project-dir に渡る。
// 未置換/空の場合はプロセスの current_dir (hook は worktree ディレクトリで実行される) にフォールバック。

// Prevents a console window flash on Windows when spawned by hooks.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

const SERVER_INFO_FILE: &str = "mcp-server.json";

/// oretachi が PTY spawn 時に発番してタブのシェルへ注入する terminal_id の env 名。
/// hook は「PTY シェル → エージェント → hook」と辿る子孫プロセスなので env を継承する。
/// CC 2.1.207 以降 hook へ oretachi 由来の情報を渡せる経路はこれだけ（`${user_config.*}` は
/// 拒否され `pluginConfigs` も読まれない）。
const TERMINAL_ID_ENV: &str = "ORETACHI_TERMINAL_ID";

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

    // SessionStart フック (--session-context): /session-context からグループの systemPrompt を
    // 取得し、SessionStart 用 JSON として stdout に出力する。
    // oretachi 非稼働・未管理ディレクトリ・プロンプト未設定のいずれでも、claude 側に警告を
    // 出さないよう何も出力せず必ず exit 0 する（--notify の exit(1) とは異なる方針）。
    if has_flag(&args, "--session-context", "-s") {
        let dir = resolve_project_dir(&args);
        let terminal_id = read_terminal_id();
        // サーバへの問い合わせが失敗しても、terminal_id だけは注入する（自己同定は
        // グループ systemPrompt の設定有無やサーバ稼働状況と独立に成立させたい）。
        let ctx = match fetch_session_context(&dir) {
            Ok(c) => c,
            Err(_e) => {
                #[cfg(debug_assertions)]
                eprintln!("Session context fetch failed: {}", _e);
                SessionContext::default()
            }
        };
        if let Some(out) = build_session_context_output(
            ctx.prompt.as_deref(),
            terminal_id.as_deref(),
            ctx.inbox.as_deref(),
        ) {
            println!("{}", out);
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

    // Stop フック (--turn-context): /turn-context からこのタブ宛の未読を取得し、
    // additionalContext JSON を stdout に出力する（#124）。Stop の additionalContext は
    // 会話を継続させるので、未読が無ければ何も出さない（＝そのままターンが終わる）。
    // --prompt-context と同じく、失敗しても何も出力せず必ず exit 0 する。
    if has_flag(&args, "--turn-context", "-t") {
        let dir = resolve_project_dir(&args);
        let hook_json = read_stdin_if_piped();
        // 継続ターンの終わりならサーバを起こさずに終わる。サーバ側でも同じ判定をするが、
        // 無駄な TCP 往復（と 2 秒の読み取り待ち）を hook のクリティカルパスから消す。
        if !should_request_turn_context(hook_json.as_deref()) {
            std::process::exit(0);
        }
        match send_turn_context(&dir, hook_json.as_deref()) {
            Ok(Some(output)) => println!("{}", output),
            Ok(None) => {}
            Err(_e) => {
                #[cfg(debug_assertions)]
                eprintln!("Turn context failed: {}", _e);
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
    eprintln!("Usage: oretachi-notify --notify --project-dir <dir> --event <Event> [--agent <agent>]\n       oretachi-notify --set-description --project-dir <dir>\n       oretachi-notify --session-context --project-dir <dir>\n       oretachi-notify --prompt-context --project-dir <dir>\n       oretachi-notify --turn-context --project-dir <dir>");
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

/// env から自分が属する oretachi ターミナルタブの terminal_id を読む。
/// oretachi 管理外のターミナルから起動された場合は未設定なので `None`。
fn read_terminal_id() -> Option<String> {
    std::env::var(TERMINAL_ID_ENV)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// payload に terminalId を付ける（未取得なら何もしない）。
fn attach_terminal_id(payload: &mut serde_json::Value, terminal_id: Option<&str>) {
    if let Some(id) = terminal_id {
        payload["terminalId"] = serde_json::Value::String(id.to_string());
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
    attach_terminal_id(&mut payload, read_terminal_id().as_deref());
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

/// /session-context のレスポンス。
/// - `prompt`: ワークツリー所属グループの systemPrompt（未管理ディレクトリ・未設定なら None）
/// - `inbox`: 購読していたワークツリーイベントの未確認メッセージ（無ければ None）
#[derive(Debug, Default, PartialEq)]
struct SessionContext {
    prompt: Option<String>,
    inbox: Option<String>,
}

/// /session-context からグループの systemPrompt と未確認の購読メッセージを取得する。
fn fetch_session_context(project_dir: &str) -> Result<SessionContext, String> {
    let mut payload = serde_json::json!({ "projectDir": project_dir });
    attach_terminal_id(&mut payload, read_terminal_id().as_deref());
    let body = post_json_read_body("/session-context", &payload)?;
    Ok(parse_session_context(&body)?)
}

/// /session-context のレスポンスボディをパースする。
fn parse_session_context(body: &str) -> Result<SessionContext, String> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("Invalid response JSON: {}", e))?;
    let field = |key: &str| {
        v.get(key)
            .and_then(|p| p.as_str())
            .map(str::to_string)
            .filter(|s| !s.trim().is_empty())
    };
    Ok(SessionContext {
        prompt: field("prompt"),
        inbox: field("inbox"),
    })
}

/// UserPromptSubmit フック用。/prompt-context から現在の description を取得し、
/// stdout に出力すべき additionalContext JSON を返す。skip 時は Ok(None)。
fn send_prompt_context(project_dir: &str) -> Result<Option<String>, String> {
    let mut payload = serde_json::json!({
        "projectDir": project_dir,
    });
    attach_terminal_id(&mut payload, read_terminal_id().as_deref());
    let body = post_json_read_body("/prompt-context", &payload)?;
    Ok(build_prompt_context_output(&body))
}

/// Stop フック用。/turn-context からこのタブ宛の未読を取得し、stdout に出力すべき
/// additionalContext JSON を返す。未読なしは Ok(None)。
fn send_turn_context(project_dir: &str, hook_json: Option<&str>) -> Result<Option<String>, String> {
    let mut payload = serde_json::json!({
        "projectDir": project_dir,
    });
    attach_terminal_id(&mut payload, read_terminal_id().as_deref());
    if let Some(j) = hook_json {
        payload["hookJson"] = serde_json::Value::String(j.to_string());
    }
    let body = post_json_read_body("/turn-context", &payload)?;
    Ok(build_turn_context_output(&body))
}

/// Stop フックの stdin JSON を見て、サーバへ問い合わせるべきか判定する（#124）。
///
/// `stop_hook_active` は「この Stop が hook 由来の継続ターンの終わりか」を CC が明示的に
/// 教えてくれるフィールド（Phase 0 / #121 で CC 2.1.227 の payload に実在することを実測）。
/// 初回発火は `false`、`additionalContext` による継続後の発火はすべて `true`。
/// **`true` のときに注入すると会話が永久に回る**ので問い合わせ自体をやめる。
///
/// stdin が無い / パースできない / フィールドが無い場合は `true`（問い合わせる）に倒す。
/// サーバ側でも同じ判定をしており、さらに `prompt_id` 単位の上限と `delivered_at` が
/// あるので、ここで安全側に倒しても暴走はしない。
fn should_request_turn_context(hook_json: Option<&str>) -> bool {
    let Some(v) = hook_json.and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok()) else {
        return true;
    };
    v.get("stop_hook_active").and_then(|b| b.as_bool()) != Some(true)
}

/// /turn-context のレスポンスボディから Stop 用の additionalContext JSON を組み立てる。
/// 未読なし・parse 失敗時は None（何も出力しない ＝ そのままターンが終わる）。
fn build_turn_context_output(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let inbox = v
        .get("inbox")
        .and_then(|i| i.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    let output = serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "Stop",
            "additionalContext": inbox,
        }
    });
    Some(output.to_string())
}

/// SessionStart 用の additionalContext JSON を組み立てる。
/// グループの systemPrompt と、自分が属するタブの terminal_id を続けて注入する。
/// terminal_id を伝えるのは、エージェントが oretachi_list_terminals 等で
/// 「自分自身のターミナル」を同定できるようにするため（同一ワークツリーに複数タブが
/// あると cwd だけでは区別できない）。いずれも無ければ `None`（何も出力しない）。
///
/// 購読メッセージ (`inbox`) を最後に置くのは、それが「今このターンで着手すべきこと」であり
/// 前段の systemPrompt / 自己 ID より新しい指示だから。
fn build_session_context_output(
    prompt: Option<&str>,
    terminal_id: Option<&str>,
    inbox: Option<&str>,
) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(p) = prompt.map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(p.to_string());
    }
    if let Some(id) = terminal_id.map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(format!(
            "[oretachi] このセッションが動いているターミナルの terminal_id は「{}」です。oretachi_list_terminals が返す terminalId と突合すれば自分自身のターミナルを同定できます（同一ワークツリーに複数タブがある場合、cwd だけでは区別できません）。",
            id
        ));
    }
    if let Some(i) = inbox.map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(i.to_string());
    }
    if parts.is_empty() {
        return None;
    }
    let output = serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "SessionStart",
            "additionalContext": parts.join("\n\n"),
        }
    });
    Some(output.to_string())
}

/// /prompt-context のレスポンスボディから UserPromptSubmit 用の
/// additionalContext JSON を組み立てる。parse 失敗時と、出すものが何も無ければ None。
///
/// `skip` は **description 側だけ**を支配する（#124）。未読の回収は 600 秒スロットルの
/// 対象外なので、`skip: true` でも `inbox` があれば出力する。`inbox` フィールドを返さない
/// 旧サーバとの後方互換は「欠落 = None」で担保する。
fn build_prompt_context_output(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let inbox = v
        .get("inbox")
        .and_then(|i| i.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if v["skip"].as_bool() == Some(true) {
        // description はスロットル中。未読だけを注入する。
        let inbox = inbox?;
        return Some(prompt_context_json(inbox));
    }
    let description = match v["description"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
        Some(desc) => format!(
            "[oretachi] このワークツリーの現在の description: 「{}」。これは作業全体の目的を表す1行です。今の作業がこの説明の範囲内（同一プランのサブタスク進行・レビュー対応など）なら更新は不要です。全く別の作業に切り替わった場合、または説明が実態と大きくずれている場合のみ oretachi_set_description ツールで更新してください。",
            desc
        ),
        None => "[oretachi] このワークツリーの description は未設定です。作業内容が決まっていれば oretachi_set_description ツールで作業全体の目的を1行でセットしてください。".to_string(),
    };
    // 未読を後ろに置くのは、それが「今このターンで着手すべきこと」であり
    // 前段の description より新しい指示だから（build_session_context_output と同じ順序）。
    let context = match inbox {
        Some(i) => format!("{}\n\n{}", description, i),
        None => description,
    };
    Some(prompt_context_json(&context))
}

fn prompt_context_json(context: &str) -> String {
    serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "UserPromptSubmit",
            "additionalContext": context,
        }
    })
    .to_string()
}

/// post_json と同様に POST し、レスポンスボディまで読んで返す。
/// /session-context・/prompt-context のようにサーバの返す JSON が必要な場合に使う。
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
    fn test_has_flag_session_context() {
        let args = vec![
            "bin".to_string(),
            "--session-context".to_string(),
            "--project-dir".to_string(),
            "X:/wt/foo".to_string(),
        ];
        assert!(has_flag(&args, "--session-context", "-s"));
        assert!(!has_flag(&args, "--prompt-context", "-c"));
        assert!(!has_flag(&args, "--notify", "-n"));
        assert!(!has_flag(&args, "--set-description", "-d"));
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
    fn test_attach_terminal_id_present() {
        let mut payload = serde_json::json!({ "projectDir": "X:/wt/foo" });
        attach_terminal_id(&mut payload, Some("11111111-2222-3333-4444-555555555555"));
        assert_eq!(payload["terminalId"], "11111111-2222-3333-4444-555555555555");
    }

    #[test]
    fn test_attach_terminal_id_absent() {
        let mut payload = serde_json::json!({ "projectDir": "X:/wt/foo" });
        attach_terminal_id(&mut payload, None);
        assert!(payload.get("terminalId").is_none());
    }

    #[test]
    fn test_build_session_context_output_prompt_only() {
        let out = build_session_context_output(Some("グループのプロンプト"), None, None).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["hookSpecificOutput"]["hookEventName"], "SessionStart");
        let ctx = v["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
        assert_eq!(ctx, "グループのプロンプト");
    }

    #[test]
    fn test_build_session_context_output_terminal_id_only() {
        // グループ systemPrompt が未設定でも terminal_id だけは注入する
        let out = build_session_context_output(None, Some("abc-123"), None).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let ctx = v["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
        assert!(ctx.contains("abc-123"));
        assert!(ctx.contains("terminal_id"));
    }

    #[test]
    fn test_build_session_context_output_both() {
        let out =
            build_session_context_output(Some("グループのプロンプト"), Some("abc-123"), None).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let ctx = v["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
        assert!(ctx.starts_with("グループのプロンプト"));
        assert!(ctx.contains("abc-123"));
    }

    #[test]
    fn test_build_session_context_output_inbox_only() {
        // グループ systemPrompt も terminal_id も無くても購読メッセージだけは注入する
        let out = build_session_context_output(None, None, Some("[oretachi] 1 件届いています")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let ctx = v["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
        assert!(ctx.contains("1 件届いています"));
    }

    #[test]
    fn test_build_session_context_output_inbox_comes_last() {
        let out = build_session_context_output(
            Some("グループのプロンプト"),
            Some("abc-123"),
            Some("[oretachi] 購読メッセージ"),
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let ctx = v["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
        assert!(ctx.starts_with("グループのプロンプト"));
        assert!(ctx.ends_with("[oretachi] 購読メッセージ"));
        // terminal_id は inbox より前
        assert!(ctx.find("abc-123").unwrap() < ctx.find("購読メッセージ").unwrap());
    }

    #[test]
    fn test_build_session_context_output_none() {
        assert_eq!(build_session_context_output(None, None, None), None);
        // 空白のみは未指定と同じ扱い
        assert_eq!(build_session_context_output(Some("  "), Some(""), Some(" ")), None);
    }

    #[test]
    fn test_parse_session_context_full() {
        let ctx = parse_session_context(r#"{"prompt":"p","inbox":"i"}"#).unwrap();
        assert_eq!(ctx.prompt.as_deref(), Some("p"));
        assert_eq!(ctx.inbox.as_deref(), Some("i"));
    }

    #[test]
    fn test_parse_session_context_nulls_and_blanks() {
        let ctx = parse_session_context(r#"{"prompt":null,"inbox":"   "}"#).unwrap();
        assert_eq!(ctx, SessionContext::default());
    }

    #[test]
    fn test_parse_session_context_missing_inbox_field() {
        // inbox を返さない旧サーバとの後方互換
        let ctx = parse_session_context(r#"{"prompt":"p"}"#).unwrap();
        assert_eq!(ctx.prompt.as_deref(), Some("p"));
        assert_eq!(ctx.inbox, None);
    }

    #[test]
    fn test_parse_session_context_invalid_json() {
        assert!(parse_session_context("not json").is_err());
    }

    #[test]
    fn test_build_prompt_context_output_skip() {
        assert_eq!(build_prompt_context_output(r#"{"skip":true}"#), None);
    }

    /// Phase 0 (#121) で実測した Stop payload そのもの。初回発火は stop_hook_active=false。
    const STOP_PAYLOAD: &str = r#"{
        "session_id": "bcbd95af-3066-4bc9-9b6b-98fabdd3ef8b",
        "transcript_path": "X:/t.jsonl",
        "cwd": "X:/wt/foo",
        "prompt_id": "46513e5d-9375-47f3-a6c0-c3a4452eb2fa",
        "permission_mode": "default",
        "effort": { "level": "medium" },
        "hook_event_name": "Stop",
        "stop_hook_active": false,
        "last_assistant_message": "2",
        "background_tasks": [],
        "session_crons": []
    }"#;

    #[test]
    fn test_has_flag_turn_context() {
        let args = vec![
            "bin".to_string(),
            "--turn-context".to_string(),
            "--project-dir".to_string(),
            "X:/wt/foo".to_string(),
        ];
        assert!(has_flag(&args, "--turn-context", "-t"));
        // 他モードと取り違えない（main の分岐順に依存しないことを固定する）
        assert!(!has_flag(&args, "--notify", "-n"));
        assert!(!has_flag(&args, "--prompt-context", "-c"));
        assert!(!has_flag(&args, "--session-context", "-s"));
        assert!(!has_flag(&args, "--set-description", "-d"));
    }

    #[test]
    fn test_should_request_turn_context_initial_firing() {
        // 初回の Stop は問い合わせる
        assert!(should_request_turn_context(Some(STOP_PAYLOAD)));
    }

    #[test]
    fn test_should_request_turn_context_blocks_continuation() {
        // additionalContext による継続後の発火。ここで注入すると無限に回る
        let json = STOP_PAYLOAD.replace("\"stop_hook_active\": false", "\"stop_hook_active\": true");
        assert!(!should_request_turn_context(Some(&json)));
    }

    #[test]
    fn test_should_request_turn_context_falls_back_to_asking() {
        // 判定材料が無いときは安全側ではなく「問い合わせる」に倒す。
        // 止めるのはサーバ側の stop_hook_active / prompt_id / delivered_at の3枚が担う。
        assert!(should_request_turn_context(None));
        assert!(should_request_turn_context(Some("not json")));
        assert!(should_request_turn_context(Some(r#"{"hook_event_name":"Stop"}"#)));
        // 文字列 "true" は bool ではないので判定材料にしない
        assert!(should_request_turn_context(Some(r#"{"stop_hook_active":"true"}"#)));
    }

    #[test]
    fn test_build_turn_context_output() {
        let out = build_turn_context_output(r#"{"inbox":"[oretachi] 1 件届いています"}"#).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["hookSpecificOutput"]["hookEventName"], "Stop");
        assert_eq!(
            v["hookSpecificOutput"]["additionalContext"],
            "[oretachi] 1 件届いています"
        );
    }

    #[test]
    fn test_build_turn_context_output_nothing_to_inject() {
        // 未読が無ければ何も出さない ＝ そのままターンが終わる（会話を継続させない）
        assert_eq!(build_turn_context_output(r#"{"inbox":null}"#), None);
        assert_eq!(build_turn_context_output(r#"{"inbox":"   "}"#), None);
        assert_eq!(build_turn_context_output(r#"{}"#), None);
        assert_eq!(build_turn_context_output("not json"), None);
    }

    #[test]
    fn test_build_prompt_context_output_inbox_survives_throttle() {
        // description は 600 秒スロットル中でも、未読だけは注入される（#120 §5.3）
        let out =
            build_prompt_context_output(r#"{"skip":true,"inbox":"[oretachi] 未読 2 件"}"#).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["hookSpecificOutput"]["hookEventName"], "UserPromptSubmit");
        let ctx = v["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
        assert_eq!(ctx, "[oretachi] 未読 2 件");
        assert!(!ctx.contains("oretachi_set_description"));
    }

    #[test]
    fn test_build_prompt_context_output_skip_without_inbox_is_none() {
        assert_eq!(build_prompt_context_output(r#"{"skip":true,"inbox":null}"#), None);
    }

    #[test]
    fn test_build_prompt_context_output_description_then_inbox() {
        let body = r#"{"description":"認証機能のリファクタリング","inbox":"[oretachi] 未読 1 件"}"#;
        let out = build_prompt_context_output(body).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let ctx = v["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
        assert!(ctx.starts_with("[oretachi] このワークツリーの現在の description"));
        assert!(ctx.ends_with("[oretachi] 未読 1 件"));
        assert!(ctx.find("認証機能").unwrap() < ctx.find("未読 1 件").unwrap());
    }

    #[test]
    fn test_build_prompt_context_output_without_inbox_field_is_unchanged() {
        // inbox を返さない旧サーバとの後方互換
        let out = build_prompt_context_output(r#"{"description":"x"}"#).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let ctx = v["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
        assert!(ctx.contains("oretachi_set_description"));
        assert!(!ctx.contains('\n'));
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
