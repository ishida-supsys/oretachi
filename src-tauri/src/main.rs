// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(name) = find_notify_arg(&args) {
        let kind = find_kind_arg(&args);
        if let Err(e) = oretachi_lib::mcp_server::send_notification_standalone(&name, kind.as_deref()) {
            #[cfg(debug_assertions)]
            eprintln!("Notification failed: {}", e);
            std::process::exit(1);
        }
        std::process::exit(0);
    }
    oretachi_lib::run()
}

fn find_notify_arg(args: &[String]) -> Option<String> {
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--notify" | "-n" => return iter.next().cloned(),
            _ if arg.starts_with("--notify=") => {
                return Some(arg["--notify=".len()..].to_string());
            }
            _ => {}
        }
    }
    None
}

fn find_kind_arg(args: &[String]) -> Option<String> {
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--kind" | "-k" => return iter.next().cloned(),
            _ if arg.starts_with("--kind=") => {
                return Some(arg["--kind=".len()..].to_string());
            }
            _ => {}
        }
    }
    None
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
}
