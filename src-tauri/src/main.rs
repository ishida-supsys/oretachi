// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(name) = find_notify_arg(&args) {
        if let Err(e) = oretachi_lib::mcp_server::send_notification_standalone(&name) {
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
