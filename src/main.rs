#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod codex_state;
mod gate;
mod installation;
mod monitor;
mod native;
mod observer;
mod probe;
mod source;
mod state;
slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--probe-observers") {
        return probe::run(&args);
    }
    if args.first().is_some_and(|a| a == "--manage-source") {
        let kind = match args.get(1).map(String::as_str) {
            Some("opencode") => source::SourceKind::Opencode,
            Some("deepseek") => source::SourceKind::Deepseek,
            _ => return Err("来源必须为 opencode 或 deepseek".into()),
        };
        let path = args.get(3).map(String::as_str).unwrap_or("");
        let info = match args.get(2).map(String::as_str) {
            Some("inspect") => installation::inspect(kind, path),
            Some("install") => installation::change(kind, path, true)?,
            Some("uninstall") => installation::change(kind, path, false)?,
            _ => return Err("操作必须为 inspect / install / uninstall".into()),
        };
        println!(
            "{}",
            serde_json::json!({"source":kind,"installed":info.installed,"note":info.note,"executable":info.executable})
        );
        return Ok(());
    }
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("femtovg-wgpu".into())
        .select()?;
    if args.iter().any(|a| a == "--native-gate") {
        gate::run(&args)
    } else if let Some(at) = args.iter().position(|a| a == "--check-navigation") {
        native::open_codex(args.get(at + 1).ok_or("缺少导航会话 ID")?)?;
        println!("{{\"urlDispatched\":true,\"taskControlsSent\":0}}");
        Ok(())
    } else {
        app::run(&args)
    }
}
