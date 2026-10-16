use crate::{observer, source::SourceKind};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const PACKAGE: &str = "@dsh-external/agmanage-observer";
const COMMON: &str = include_str!("../plugins/common/observer.mjs");
#[derive(Clone, Debug)]
pub struct Info {
    pub executable: String,
    pub installed: bool,
    pub note: String,
}
pub fn config_home() -> PathBuf {
    std::env::var_os("OPENCODE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default())
                        .join(".config")
                })
                .join("opencode")
        })
}
fn dsh_profile() -> PathBuf {
    std::env::var_os("DSH_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default()).join(".dsh")
        })
        .join("profiles/desktop")
}
fn configs() -> Vec<PathBuf> {
    let mut paths = ["config.json", "opencode.json", "opencode.jsonc"]
        .map(|name| config_home().join(name))
        .to_vec();
    if let Some(path) = std::env::var_os("OPENCODE_CONFIG") {
        paths.push(PathBuf::from(path));
    }
    paths
}
fn legacy_home() -> PathBuf {
    PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("AgManage")
}
fn detect(kind: SourceKind) -> PathBuf {
    for root in [observer::home(), legacy_home()] {
        let saved = root.join(format!("installation-{}.json", kind.code()));
        if let Ok(bytes) = fs::read(saved)
            && let Ok(v) = serde_json::from_slice::<Value>(&bytes)
            && let Some(path) = v["executable"].as_str()
            && Path::new(path).is_file()
        {
            return path.into();
        }
    }
    let local = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default());
    let program = PathBuf::from(std::env::var_os("PROGRAMFILES").unwrap_or_default());
    let candidates = match kind {
        SourceKind::Opencode => vec![
            local.join("Programs/@opencode-aidesktop/OpenCode.exe"),
            local.join("Programs/@opencode-aidesktop/opencode.exe"),
            program.join("OpenCode/OpenCode.exe"),
        ],
        SourceKind::Deepseek => vec![
            local.join("Programs/DeepSeek Harness/DeepSeek Harness.exe"),
            program.join("DeepSeek Harness/DeepSeek Harness.exe"),
            PathBuf::from(r"C:\ProgramSoftware\Deepseek\DeepSeek Harness.exe"),
        ],
        SourceKind::Codex => Vec::new(),
    };
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .unwrap_or_default()
}
fn package_version(executable: &Path, kind: SourceKind) -> Result<String, &'static str> {
    let path = executable
        .parent()
        .ok_or("请选择桌面程序的 exe 文件")?
        .join("resources/app.asar");
    let mut file = fs::File::open(path).map_err(|_| "未找到桌面安装包 resources/app.asar")?;
    let mut head = [0u8; 16];
    file.read_exact(&mut head).map_err(|_| "安装包头无效")?;
    let size = u32::from_le_bytes(head[12..16].try_into().unwrap()) as usize;
    if size > 32 * 1024 * 1024 {
        return Err("安装包目录过大");
    }
    let mut bytes = vec![0; size];
    file.read_exact(&mut bytes)
        .map_err(|_| "无法读取安装包目录")?;
    let header: Value = serde_json::from_slice(&bytes).map_err(|_| "安装包目录格式不支持")?;
    let entry = &header["files"]["package.json"];
    let offset = entry["offset"]
        .as_str()
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or("安装包缺少版本信息")?;
    let size = entry["size"]
        .as_u64()
        .filter(|n| *n < 1024 * 1024)
        .ok_or("安装包版本信息无效")? as usize;
    file.seek(SeekFrom::Start(
        8 + u32::from_le_bytes(head[4..8].try_into().unwrap()) as u64 + offset,
    ))
    .map_err(|_| "无法定位安装包版本")?;
    bytes.resize(size, 0);
    file.read_exact(&mut bytes)
        .map_err(|_| "无法读取安装包版本")?;
    let package: Value = serde_json::from_slice(&bytes).map_err(|_| "安装包版本格式无效")?;
    let expected = match kind {
        SourceKind::Opencode => "@opencode-ai/desktop",
        SourceKind::Deepseek => "@deepseek-ai/dsh-desktop",
        _ => return Err("来源不需要插件"),
    };
    if package["name"] != expected {
        return Err("选择的程序不是目标桌面应用");
    }
    package["version"]
        .as_str()
        .map(str::to_owned)
        .ok_or("安装包缺少版本号")
}
fn owned_entry() -> String {
    opencode_entry(&observer::home())
}
fn opencode_entry(root: &Path) -> String {
    let path = root.join("plugins/opencode/index.mjs");
    let path = path.to_string_lossy().replace('\\', "/");
    let mut url = String::from("file:///");
    for byte in path.as_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~:".contains(byte) {
            url.push(*byte as char);
        } else {
            url.push_str(&format!("%{byte:02X}"));
        }
    }
    url
}

// 保持字节位置不变，屏蔽注释和尾逗号；只用于检查及定位，不用重写整份用户配置。
fn mask_comments(source: &str) -> Result<Vec<u8>, &'static str> {
    let mut bytes = source.as_bytes().to_vec();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i += 2;
                } else if bytes[i] == b'"' {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
        } else if bytes.get(i..i + 2) == Some(b"//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                bytes[i] = b' ';
                i += 1;
            }
        } else if bytes.get(i..i + 2) == Some(b"/*") {
            bytes[i] = b' ';
            bytes[i + 1] = b' ';
            i += 2;
            while i + 1 < bytes.len() && &bytes[i..i + 2] != b"*/" {
                bytes[i] = b' ';
                i += 1;
            }
            if i + 1 >= bytes.len() {
                return Err("配置注释未闭合");
            }
            bytes[i] = b' ';
            bytes[i + 1] = b' ';
            i += 2;
        } else {
            i += 1;
        }
    }
    Ok(bytes)
}
fn mask_jsonc(source: &str) -> Result<Vec<u8>, &'static str> {
    let mut bytes = mask_comments(source)?;
    let mut i = 0;
    let mut quoted = false;
    while i < bytes.len() {
        if quoted {
            if bytes[i] == b'\\' {
                i += 2;
                continue;
            }
            if bytes[i] == b'"' {
                quoted = false;
            }
        } else if bytes[i] == b'"' {
            quoted = true;
        } else if bytes[i] == b',' {
            let next = bytes[i + 1..].iter().find(|b| !b.is_ascii_whitespace());
            if matches!(next, Some(b']' | b'}')) {
                bytes[i] = b' ';
            }
        }
        i += 1;
    }
    Ok(bytes)
}
fn parse_jsonc(source: &str) -> Result<Value, &'static str> {
    serde_json::from_slice(&mask_jsonc(source)?)
        .map_err(|_| "OpenCode 配置格式无效，请先在原应用修复")
}
fn value_end(mask: &[u8], start: usize) -> Result<usize, &'static str> {
    let mut depth = 0i32;
    let mut quoted = false;
    let mut i = start;
    while i < mask.len() {
        let b = mask[i];
        if quoted {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == b'"' {
                quoted = false;
                if depth == 0 {
                    return Ok(i + 1);
                }
            }
        } else {
            match b {
                b'"' => quoted = true,
                b'[' | b'{' => depth += 1,
                b']' | b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(i + 1);
                    }
                    if depth < 0 {
                        return Ok(i);
                    }
                }
                b',' if depth == 0 => return Ok(i),
                _ => {}
            }
        }
        i += 1;
    }
    Err("配置值未闭合")
}
fn property_span(source: &str) -> Result<Option<(usize, usize)>, &'static str> {
    let mask = mask_jsonc(source)?;
    let mut i = mask
        .iter()
        .position(|b| *b == b'{')
        .ok_or("配置必须为对象")?
        + 1;
    loop {
        while i < mask.len() && (mask[i].is_ascii_whitespace() || mask[i] == b',') {
            i += 1;
        }
        if mask.get(i) == Some(&b'}') {
            return Ok(None);
        }
        let end = value_end(&mask, i)?;
        let key: String = serde_json::from_slice(&mask[i..end]).map_err(|_| "配置键格式错误")?;
        i = end;
        while mask.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        if mask.get(i) != Some(&b':') {
            return Err("配置键缺少冒号");
        }
        i += 1;
        while mask.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        let end = value_end(&mask, i)?;
        if key == "plugin" {
            return Ok(Some((i, end)));
        }
        i = end;
    }
}
pub fn edit_registration(source: &str, entry: &str, install: bool) -> Result<String, &'static str> {
    let document = parse_jsonc(source)?;
    if !document.is_object() {
        return Err("配置必须为对象");
    }
    let encoded = serde_json::to_string(entry).unwrap();
    if let Some((start, end)) = property_span(source)? {
        let array = document["plugin"].as_array().ok_or("plugin 必须为数组")?;
        let exists = array.iter().any(|v| v.as_str() == Some(entry));
        if exists == install {
            return Ok(source.into());
        }
        if install {
            let masked = mask_comments(source)?;
            let mut last_end = None;
            let mut pos = start + 1;
            while pos < end - 1 {
                while pos < end - 1 && (masked[pos].is_ascii_whitespace() || masked[pos] == b',') {
                    pos += 1;
                }
                if pos >= end - 1 {
                    break;
                }
                pos = value_end(&masked, pos)?;
                last_end = Some(pos);
            }
            let mut result = source.to_string();
            let mut closing = end - 1;
            if let Some(last) = last_end {
                if !masked[last..end - 1].contains(&b',') {
                    result.insert(last, ',');
                    closing += 1;
                }
            }
            result.insert_str(closing, &format!("\n    {encoded}\n  "));
            parse_jsonc(&result)?;
            Ok(result)
        } else {
            let masked = mask_comments(source)?;
            let mut pos = start + 1;
            while pos < end - 1 {
                while pos < end - 1 && (masked[pos].is_ascii_whitespace() || masked[pos] == b',') {
                    pos += 1;
                }
                if pos >= end - 1 {
                    break;
                }
                let stop = value_end(&masked, pos)?;
                if serde_json::from_slice::<Value>(&masked[pos..stop])
                    .ok()
                    .is_some_and(|v| v.as_str() == Some(entry))
                {
                    let mut bytes = source.as_bytes().to_vec();
                    let mut next = stop;
                    while next < end - 1 && masked[next].is_ascii_whitespace() {
                        next += 1;
                    }
                    if masked.get(next) == Some(&b',') {
                        bytes[next] = b' ';
                    } else {
                        let mut prev = pos;
                        while prev > start + 1 {
                            prev -= 1;
                            if masked[prev] == b',' {
                                bytes[prev] = b' ';
                                break;
                            }
                            if !masked[prev].is_ascii_whitespace() {
                                break;
                            }
                        }
                    }
                    bytes[pos..stop].fill(b' ');
                    let result = String::from_utf8(bytes).map_err(|_| "配置编码错误")?;
                    parse_jsonc(&result)?;
                    return Ok(result);
                }
                pos = stop;
            }
            Ok(source.into())
        }
    } else if install {
        let close = source.rfind('}').ok_or("配置缺少结束括号")?;
        let mut result = source.to_string();
        let comma = if document.as_object().unwrap().is_empty() {
            ""
        } else {
            ","
        };
        // 原对象已有尾逗号时插入新键无需额外逗号。
        let prefix = &source[..close];
        let no_comments = mask_comments(prefix)?;
        let last = no_comments
            .iter()
            .rposition(|b| !b.is_ascii_whitespace())
            .unwrap_or(0);
        let comma = if prefix.as_bytes().get(last) == Some(&b',') {
            ""
        } else {
            comma
        };
        result.insert_str(close, &format!("{comma}\n  \"plugin\": [{encoded}]\n"));
        parse_jsonc(&result)?;
        Ok(result)
    } else {
        Ok(source.into())
    }
}
fn atomic_update(path: &Path, before: &str, after: &str) -> Result<(), &'static str> {
    if before == after {
        return Ok(());
    }
    let parent = path.parent().ok_or("配置目录无效")?;
    fs::create_dir_all(parent).map_err(|_| "无法创建配置目录")?;
    if path.exists() && fs::read_to_string(path).map_err(|_| "无法重读配置")? != before {
        return Err("配置已被其他程序修改，请重试");
    }
    let temporary = parent.join(format!(".agmanage-{}.tmp", uuid::Uuid::new_v4()));
    fs::write(&temporary, after).map_err(|_| "无法写入配置临时文件")?;
    if let Err(_) = fs::rename(&temporary, path) {
        fs::remove_file(&temporary).ok();
        return Err("无法替换配置文件");
    }
    Ok(())
}
fn register_opencode(install: bool) -> Result<(), &'static str> {
    let entry = owned_entry();
    let legacy = opencode_entry(&legacy_home());
    let paths = configs();
    let mut chosen = None;
    for path in &paths {
        if path.is_file() {
            let source = fs::read_to_string(path).map_err(|_| "无法读取 OpenCode 配置")?;
            let value = parse_jsonc(&source)?;
            if value.get("plugin").is_some() {
                chosen = Some(path.clone());
            }
        }
    }
    if install {
        let path = chosen
            .or_else(|| paths.iter().rev().find(|p| p.is_file()).cloned())
            .unwrap_or_else(|| config_home().join("opencode.json"));
        let before = if path.is_file() {
            fs::read_to_string(&path).map_err(|_| "无法读取 OpenCode 配置")?
        } else {
            "{}\n".into()
        };
        let after = edit_registration(&edit_registration(&before, &legacy, false)?, &entry, true)?;
        atomic_update(&path, &before, &after)?;
    }
    // 只移除旧目录中本项目的入口，防止迁移后重复加载；其他插件及旧文件均保留。
    for path in paths {
        if path.is_file() {
            let before = fs::read_to_string(&path).map_err(|_| "无法读取 OpenCode 配置")?;
            let mut after = edit_registration(&before, &legacy, false)?;
            if !install {
                after = edit_registration(&after, &entry, false)?;
            }
            atomic_update(&path, &before, &after)?;
        }
    }
    Ok(())
}
fn is_registered(kind: SourceKind) -> Result<bool, &'static str> {
    if kind == SourceKind::Opencode {
        let mut installed = false;
        for path in configs() {
            if path.is_file() {
                let source = fs::read_to_string(path).map_err(|_| "无法读取 OpenCode 配置")?;
                let value = parse_jsonc(&source)?;
                if let Some(plugins) = value.get("plugin") {
                    installed = plugins
                        .as_array()
                        .ok_or("plugin 必须为数组")?
                        .iter()
                        .any(|v| v.as_str() == Some(&owned_entry()));
                }
            }
        }
        Ok(installed)
    } else {
        let path = dsh_profile().join("package.json");
        if !path.is_file() {
            return Ok(false);
        }
        let value: Value =
            serde_json::from_slice(&fs::read(path).map_err(|_| "无法读取桌面插件清单")?)
                .map_err(|_| "桌面插件清单无效")?;
        Ok(value["dependencies"].get(PACKAGE).is_some())
    }
}
fn verify_harness_installation(profile: &Path) -> Result<(), &'static str> {
    let manifest: Value = serde_json::from_slice(
        &fs::read(profile.join("package.json")).map_err(|_| "无法读取桌面插件清单")?,
    )
    .map_err(|_| "桌面插件清单无效")?;
    if !manifest["dsh"]["profile"]["bundles"]
        .as_array()
        .is_some_and(|bundles| {
            bundles
                .iter()
                .any(|bundle| bundle.as_str() == Some(PACKAGE))
        })
    {
        return Err("观察插件尚未启用，请在 Harness 插件页启用");
    }
    // 清单中的依赖声明不能证明桌面加载器可以读取包；检查实际安装位置。
    let package = profile.join("node_modules").join(PACKAGE);
    for (name, expected) in plugin_files(SourceKind::Deepseek) {
        let installed = fs::read(package.join(name))
            .map_err(|_| "观察插件已登记，但依赖包或文件缺失；请点击更新修复")?;
        if installed != expected.as_bytes() {
            return Err("观察插件文件与当前版本不一致；请点击更新修复");
        }
    }
    Ok(())
}
pub fn inspect(kind: SourceKind, selected: &str) -> Info {
    let path = if selected.trim().is_empty() {
        detect(kind)
    } else {
        PathBuf::from(selected.trim())
    };
    let registration = is_registered(kind);
    let installed = registration.as_ref().copied().unwrap_or(false);
    let note = match package_version(&path, kind) {
        Ok(version) if version == kind.supported_version() => {
            if let Err(code) = registration {
                code.into()
            } else if installed && kind == SourceKind::Deepseek {
                match verify_harness_installation(&dsh_profile()) {
                    Ok(()) => format!("插件文件已校验 · {version}；等待原应用连接"),
                    Err(code) => code.into(),
                }
            } else if installed {
                format!("已注册插件 · {version}；等待原应用连接")
            } else {
                format!("已检测到 {version} · 尚未安装观察插件")
            }
        }
        Ok(version) => format!("不兼容版本 {version}；支持 {}", kind.supported_version()),
        Err(code) => code.into(),
    };
    Info {
        executable: path.to_string_lossy().into(),
        installed,
        note,
    }
}
fn plugin_files(kind: SourceKind) -> Vec<(&'static str, &'static str)> {
    if kind == SourceKind::Opencode {
        vec![
            ("common/observer.mjs", COMMON),
            (
                "opencode/source.mjs",
                include_str!("../plugins/opencode/source.mjs"),
            ),
            (
                "opencode/index.mjs",
                include_str!("../plugins/opencode/index.mjs"),
            ),
        ]
    } else {
        vec![
            ("observer.mjs", COMMON),
            ("index.mjs", include_str!("../plugins/deepseek/index.mjs")),
            (
                "package.json",
                include_str!("../plugins/deepseek/package.json"),
            ),
            (
                "cordis.patch.yml",
                include_str!("../plugins/deepseek/cordis.patch.yml"),
            ),
        ]
    }
}
fn materialize(kind: SourceKind) -> Result<PathBuf, &'static str> {
    use std::hash::{Hash, Hasher};
    let files = plugin_files(kind);
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    files.hash(&mut hash);
    let root = if kind == SourceKind::Deepseek {
        observer::home().join(format!("plugins/deepseek/{:016x}", hash.finish()))
    } else {
        observer::home().join("plugins")
    };
    for (path, content) in files {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).map_err(|_| "无法创建插件目录")?;
        fs::write(path, content).map_err(|_| "无法写入观察插件")?;
    }
    Ok(root)
}
fn remove_owned_files(kind: SourceKind) -> Result<(), &'static str> {
    let record = observer::home().join(format!("installation-{}.json", kind.code()));
    let value: Value = fs::read(record)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    let Some(directory) = value["pluginDirectory"].as_str() else {
        return Ok(());
    };
    let root = PathBuf::from(directory);
    let allowed = observer::home().join("plugins");
    let canonical = root.canonicalize().map_err(|_| "插件目录不可用")?;
    let boundary = allowed.canonicalize().map_err(|_| "插件目录不可用")?;
    if !canonical.starts_with(&boundary) {
        return Err("插件目录不属于本程序，已保留文件");
    }
    for (name, expected) in plugin_files(kind) {
        let path = root.join(name);
        if fs::read_to_string(&path).ok().as_deref() == Some(expected) {
            fs::remove_file(path).map_err(|_| "无法删除本项目插件文件")?;
        }
    }
    if kind == SourceKind::Opencode {
        fs::remove_dir(root.join("opencode")).ok();
        fs::remove_dir(root.join("common")).ok();
    } else {
        fs::remove_dir(root).ok();
    }
    Ok(())
}

fn harness_command(executable: &Path, action: &str, target: &str) -> Result<(), &'static str> {
    let entry = executable
        .parent()
        .ok_or("桌面程序路径无效")?
        .join("resources/app.asar/dsh/node_modules/@deepseek-ai/dsh-desktop-host/lib/cli.js");
    let mut child = Command::new(executable)
        .env("ELECTRON_RUN_AS_NODE", "1")
        .args(["--expose-internals"])
        .arg(entry)
        .args([
            "plugin",
            "--profile",
            "desktop",
            action,
            target,
            "--ignore-scripts",
        ])
        .creation_flags(0x08000000)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "无法启动桌面自带插件管理器")?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    Ok(())
                } else {
                    Err("桌面插件管理器安装失败；请在原应用插件页检查")
                };
            }
            Ok(None) => {}
            Err(_) => return Err("无法读取插件管理器退出状态"),
        }
        if start.elapsed() > std::time::Duration::from_secs(180) {
            let _ = Command::new("taskkill.exe")
                .args(["/PID", &child.id().to_string(), "/T", "/F"])
                .creation_flags(0x08000000)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            return Err("插件管理操作超时；已终止本次安装子进程，请检查插件页");
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
pub fn change(kind: SourceKind, selected: &str, install: bool) -> Result<Info, &'static str> {
    let path = if selected.trim().is_empty() {
        detect(kind)
    } else {
        PathBuf::from(selected.trim())
    };
    if package_version(&path, kind)? != kind.supported_version() {
        return Err("宿主版本不兼容，未修改插件配置");
    }
    let before = is_registered(kind)?;
    let mut plugin_directory = None;
    if install {
        let root = materialize(kind)?;
        plugin_directory = Some(root.clone());
        match kind {
            SourceKind::Opencode => register_opencode(true)?,
            SourceKind::Deepseek => {
                harness_command(&path, "add", &root.to_string_lossy())?;
                verify_harness_installation(&dsh_profile())?;
            }
            _ => return Err("来源不需要插件"),
        }
    } else {
        match kind {
            SourceKind::Opencode => register_opencode(false)?,
            SourceKind::Deepseek if before => harness_command(&path, "remove", PACKAGE)?,
            _ => {}
        }
    }
    if !install {
        remove_owned_files(kind)?;
    }
    fs::create_dir_all(observer::home()).map_err(|_| "无法保存接入设置")?;
    fs::write(
        observer::home().join(format!("installation-{}.json", kind.code())),
        serde_json::to_vec(
            &json!({"executable":path,"installed":install,"pluginDirectory":plugin_directory}),
        )
        .unwrap(),
    )
    .map_err(|_| "无法保存接入设置")?;
    let mut info = inspect(kind, &path.to_string_lossy());
    if info.installed != install {
        return Err("插件注册结果与请求不一致，请检查原应用插件页");
    }
    info.note = if install {
        if kind == SourceKind::Opencode {
            "插件已注册或更新；请在方便时重新打开原应用以加载"
        } else {
            "插件文件已校验；请在 Harness 插件页重新启用观察插件，必要时重开应用"
        }
    } else {
        "已移除本项目注册项；已加载的旧插件可能需要重启原应用后退出"
    }
    .into();
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn harness_registration_requires_readable_current_package() {
        let temporary = std::env::temp_dir();
        let profile = temporary.join(format!("agmanage-install-check-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&profile).unwrap();
        let manifest = json!({
            "dependencies": { PACKAGE: "link:fixture" },
            "dsh": { "profile": { "bundles": [PACKAGE] } }
        });
        fs::write(profile.join("package.json"), manifest.to_string()).unwrap();
        assert!(
            verify_harness_installation(&profile)
                .unwrap_err()
                .contains("缺失")
        );
        let package = profile.join("node_modules").join(PACKAGE);
        fs::create_dir_all(&package).unwrap();
        for (name, content) in plugin_files(SourceKind::Deepseek) {
            fs::write(package.join(name), content).unwrap();
        }
        assert_eq!(verify_harness_installation(&profile), Ok(()));
        fs::write(package.join("index.mjs"), "旧版本").unwrap();
        assert!(
            verify_harness_installation(&profile)
                .unwrap_err()
                .contains("不一致")
        );
        let mut disabled = manifest;
        disabled["dsh"]["profile"]["bundles"] = json!([]);
        fs::write(profile.join("package.json"), disabled.to_string()).unwrap();
        assert!(
            verify_harness_installation(&profile)
                .unwrap_err()
                .contains("未启用")
        );
        let target = profile.canonicalize().unwrap();
        assert!(target.starts_with(temporary.canonicalize().unwrap()));
        fs::remove_dir_all(target).unwrap();
    }
    #[test]
    fn registration_preserves_foreign_values_comments_and_is_idempotent() {
        for source in [
            "{\n// 中文说明\n\"theme\":\"dark\",\"plugin\":[\"foreign\"]\n}",
            "{\"plugin\": [\"foreign\", /*保留注释*/],}",
            "{\"theme\":\"dark\"}",
            "{\"theme\":\"dark\", // 注释\n}",
            "{\"plugin\":[]}",
            "{}",
        ] {
            let added = edit_registration(source, "file:///owned/index.mjs", true).unwrap();
            assert!(
                parse_jsonc(&added).unwrap()["plugin"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v == "file:///owned/index.mjs")
            );
            assert_eq!(
                edit_registration(&added, "file:///owned/index.mjs", true).unwrap(),
                added
            );
            let removed = edit_registration(&added, "file:///owned/index.mjs", false).unwrap();
            assert!(!removed.contains("file:///owned"));
            if source.contains("foreign") {
                assert!(removed.contains("foreign"));
            }
            if source.contains("保留注释") {
                assert!(removed.contains("保留注释"));
            }
        }
    }
}
