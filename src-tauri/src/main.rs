// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(name) = find_notify_arg(&args) {
        let kind = find_kind_arg(&args);
        let agent = find_agent_arg(&args);
        let body = read_stdin_if_piped();
        if let Err(e) = oretachi_lib::mcp_server::send_notification_standalone(
            &name,
            kind.as_deref(),
            body.as_deref(),
            agent.as_deref(),
        ) {
            #[cfg(debug_assertions)]
            eprintln!("Notification failed: {}", e);
            std::process::exit(1);
        }
        std::process::exit(0);
    }
    oretachi_lib::run()
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
        let _ = std::io::stdin().take(65536).read_to_string(&mut buf);
        let _ = tx.send(buf);
    });
    match rx.recv_timeout(std::time::Duration::from_secs(2)) {
        Ok(s) if !s.is_empty() => Some(s),
        _ => None,
    }
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
}
