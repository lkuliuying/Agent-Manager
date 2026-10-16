use crate::NativeGate;
use serde_json::json;
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::{cell::Cell, path::Path, rc::Rc, time::Duration};

pub fn save_snapshot(
    window: &slint::Window,
    path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let pixels = window.take_snapshot()?;
    let file = std::fs::File::create(path)?;
    let mut encoder = png::Encoder::new(file, pixels.width(), pixels.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()?
        .write_image_data(pixels.as_bytes())?;
    Ok(())
}

fn clipboard_text() -> Option<String> {
    use windows_sys::Win32::System::{DataExchange::*, Memory::*};
    // 只在本验证窗口复制演示文本后读取，不读取启动前的剪贴板内容。
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return None;
        }
        let handle = GetClipboardData(13);
        let result = if !handle.is_null() {
            let ptr = GlobalLock(handle) as *const u16;
            if ptr.is_null() {
                None
            } else {
                let mut len = 0;
                while *ptr.add(len) != 0 {
                    len += 1;
                }
                let text = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
                GlobalUnlock(handle);
                Some(text)
            }
        } else {
            None
        };
        CloseClipboard();
        result
    }
}

pub fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let dir = args
        .iter()
        .position(|a| a == "--gate-output")
        .and_then(|at| args.get(at + 1))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "artifacts/native-gate".into());
    std::fs::create_dir_all(&dir)?;
    let ui = NativeGate::new()?;
    let sample = ui.get_sample().to_string();
    ui.set_long_output(
        (0..600)
            .map(|i| format!("{i:04}  中文工具输出 · 会话更新仍保持独立  [status: running]\n"))
            .collect::<String>()
            .into(),
    );
    let streams = Rc::new(VecModel::from(vec![
        SharedString::from("会话 A · 0"),
        SharedString::from("会话 B · 0"),
        SharedString::from("会话 C · 0"),
    ]));
    ui.set_streams(ModelRc::from(streams.clone()));
    let tick = Rc::new(Cell::new(0));
    let timer = Timer::default();
    let weak = ui.as_weak();
    let result = Rc::new(Cell::new(false));
    let checked = result.clone();
    let selected = Rc::new(Cell::new(false));
    let isolated = Rc::new(Cell::new(true));
    timer.start(TimerMode::Repeated, Duration::from_millis(180), move || {
        use slint::Model;
        let Some(ui) = weak.upgrade() else { return; };
        let n = tick.get() + 1;
        tick.set(n);
        let before_rows:Vec<_>=(0..3).map(|i|streams.row_data(i).unwrap()).collect();
        streams.set_row_data((n % 3) as usize, format!("会话 {} · 更新 {n}", ['A','B','C'][(n%3) as usize]).into());
        isolated.set(isolated.get()&&(0..3).filter(|i|streams.row_data(*i).as_ref()!=Some(&before_rows[*i])).count()==1);
        if n==1 {
            use slint::platform::{WindowEvent,PointerEventButton};
            let y=ui.get_sample_hit_y();
            ui.window().dispatch_event(WindowEvent::PointerPressed{position:slint::LogicalPosition::new(40.,y),button:PointerEventButton::Left});
            ui.window().dispatch_event(WindowEvent::PointerMoved{position:slint::LogicalPosition::new(240.,y)});
            ui.window().dispatch_event(WindowEvent::PointerReleased{position:slint::LogicalPosition::new(240.,y),button:PointerEventButton::Left});
            ui.invoke_copy_selection();
            selected.set(clipboard_text().is_some_and(|value|!value.is_empty()&&value.len()<sample.len()&&sample.contains(&value)));
        }
        if n == 3 {
            use slint::platform::{WindowEvent,Key};
            ui.invoke_focus_sample();
            ui.window().dispatch_event(WindowEvent::KeyPressed{text:Key::Control.into()});
            for key in ["a","c"] {ui.window().dispatch_event(WindowEvent::KeyPressed{text:key.into()});ui.window().dispatch_event(WindowEvent::KeyReleased{text:key.into()});}
            ui.window().dispatch_event(WindowEvent::KeyReleased{text:Key::Control.into()});
            let copied = clipboard_text();
            let copy_ok = copied.as_ref().is_some_and(|text| text.replace("\r\n", "\n") == sample);
            ui.invoke_focus_sample();
            // 通过原生窗口事件验证只读正文不能被输入覆盖。
            ui.window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: "x".into() });
            ui.window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text: "x".into() });
            let readonly_ok = ui.get_sample().as_str() == sample;
            let before = ui.get_scroll_y();
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                position: slint::LogicalPosition::new(400., ui.get_scroll_hit_y()), delta_x: 0., delta_y: -420.,
            });
            let scroll_ok = ui.get_scroll_y() < before;
            let capture = save_snapshot(ui.window(), &dir.join("native.png"));
            let report = json!({"backend":"winit-femtovg-wgpu", "scaleFactor":ui.window().scale_factor(), "scaleOverride":std::env::var("SLINT_SCALE_FACTOR").ok(), "mouseSelectionCopied":selected.get(), "clipboardChineseAndCode":copy_ok, "clipboardAvailable":copied.is_some(), "clipboardCharacters":copied.as_ref().map(|s|s.chars().count()), "readOnlyInput":readonly_ok, "longOutputScrolled":scroll_ok, "scrollBefore":before, "scrollAfter":ui.get_scroll_y(), "threeIndependentModels":streams.row_count()==3&&isolated.get(), "snapshot":capture.is_ok(), "syntheticOnly":true});
            let pass = copy_ok && selected.get() && isolated.get() && readonly_ok && scroll_ok && capture.is_ok();
            std::fs::write(dir.join("result.json"), serde_json::to_vec_pretty(&report).unwrap()).expect("无法保存能力验证结果");
            println!("{report}");
            checked.set(pass);
        }
        if n == 5 { slint::quit_event_loop().ok(); }
    });
    ui.run()?;
    if !result.get() {
        return Err("原生能力验证未通过，见 artifacts/native-gate/result.json".into());
    }
    Ok(())
}
