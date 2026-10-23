//! 從 GitHub Releases 下載並安裝 oneechk 核心（oneechk.exe）。
//!
//! 下載與解壓縮優先使用 Windows 內建的 curl.exe / tar.exe；
//! 失敗時改用 PowerShell 的 Invoke-WebRequest / Expand-Archive 備援，
//! 因此不需要額外的執行期依賴。

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;

const DEFAULT_REPO: &str = "XiaoYao-Www/onee_checker";
const WINDOWS_ASSET_SUFFIX: &str = "-x86_64-pc-windows-gnu.zip";
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 安裝面板的狀態與背景工作通道。
pub struct Installer {
    pub repo: String,
    pub tag: String,
    pub target_dir: String,
    pub status: String,
    pub log: String,
    busy: bool,
    last_error: bool,
    progress_bytes: Option<u64>,
    receiver: Option<Receiver<InstallEvent>>,
}

enum InstallEvent {
    Log(String),
    Progress(u64),
    Done { path: PathBuf, version: String },
    Failed(String),
}

impl Default for Installer {
    fn default() -> Self {
        Self::new()
    }
}

impl Installer {
    pub fn new() -> Self {
        let target_dir = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf))
            .map_or_else(String::new, |path| path.display().to_string());
        Self {
            repo: DEFAULT_REPO.to_owned(),
            tag: String::new(),
            target_dir,
            status: "可從 GitHub 下載並安裝 oneechk 核心（需要網路）。".to_owned(),
            log: String::new(),
            busy: false,
            last_error: false,
            progress_bytes: None,
            receiver: None,
        }
    }

    pub fn is_busy(&self) -> bool {
        self.busy
    }

    fn start(&mut self) {
        if self.busy {
            return;
        }
        let repo = self.repo.trim().to_owned();
        let tag = self.tag.trim().to_owned();
        let target_dir = self.target_dir.trim().to_owned();
        let repo_valid = repo
            .split_once('/')
            .is_some_and(|(owner, name)| !owner.trim().is_empty() && !name.trim().is_empty());
        if !repo_valid {
            self.status = "repo 格式應為 owner/repo，例如 XiaoYao-Www/onee_checker。".to_owned();
            self.last_error = true;
            return;
        }
        if target_dir.is_empty() {
            self.status = "請先指定安裝資料夾。".to_owned();
            self.last_error = true;
            return;
        }

        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.busy = true;
        self.last_error = false;
        self.progress_bytes = None;
        self.log.clear();
        self.status = "正在向 GitHub 查詢 release…".to_owned();

        thread::spawn(move || {
            let sender_for_log = sender.clone();
            let mut note = move |line: String| {
                let _ = sender_for_log.send(InstallEvent::Log(line));
            };
            match install(&repo, &tag, Path::new(&target_dir), &sender, &mut note) {
                Ok((path, version)) => {
                    let _ = sender.send(InstallEvent::Done { path, version });
                }
                Err(error) => {
                    let _ = sender.send(InstallEvent::Failed(error));
                }
            }
        });
    }

    /// 每幀呼叫；若安裝完成則回傳新的 oneechk.exe 路徑。
    pub fn poll(&mut self) -> Option<PathBuf> {
        let receiver = self.receiver.take()?;
        let mut installed = None;
        let mut finished = false;
        loop {
            match receiver.try_recv() {
                Ok(InstallEvent::Log(line)) => {
                    if !self.log.is_empty() {
                        self.log.push('\n');
                    }
                    self.log.push_str(&line);
                    self.status = line;
                }
                Ok(InstallEvent::Progress(bytes)) => {
                    self.progress_bytes = Some(bytes);
                }
                Ok(InstallEvent::Done { path, version }) => {
                    self.busy = false;
                    self.last_error = false;
                    self.progress_bytes = None;
                    self.status = if version.is_empty() {
                        format!("已安裝：{}", path.display())
                    } else {
                        format!("已安裝 {version}：{}", path.display())
                    };
                    installed = Some(path);
                    finished = true;
                    break;
                }
                Ok(InstallEvent::Failed(message)) => {
                    self.busy = false;
                    self.last_error = true;
                    self.progress_bytes = None;
                    self.status = format!("安裝失敗：{message}");
                    finished = true;
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.busy = false;
                    self.last_error = true;
                    self.progress_bytes = None;
                    self.status = "安裝背景工作已中斷，沒有收到結果。".to_owned();
                    finished = true;
                    break;
                }
            }
        }
        if !finished {
            self.receiver = Some(receiver);
        }
        installed
    }

    pub fn draw(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("從 GitHub 安裝核心")
            .default_open(true)
            .show(ui, |ui| {
                ui.small("從 GitHub Releases 下載 Windows 版 oneechk.exe，安裝到指定資料夾；下載需要網路。");

                ui.horizontal(|ui| {
                    ui.label("GitHub repo");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.repo)
                            .desired_width(f32::INFINITY)
                            .hint_text("owner/repo"),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("版本 tag");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.tag)
                            .desired_width(f32::INFINITY)
                            .hint_text("留空＝最新正式版；也可輸入例如 v2.0.3_Auto"),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("安裝資料夾");
                    let width = (ui.available_width() - 230.0).max(140.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.target_dir)
                            .desired_width(width)
                            .hint_text("例如 UI 執行檔所在資料夾"),
                    );
                    if ui.button("選擇資料夾…").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            self.target_dir = path.display().to_string();
                        }
                    }
                    if ui.button("開啟資料夾").clicked() {
                        self.open_folder();
                    }
                });

                ui.horizontal(|ui| {
                    let button = egui::Button::new(if self.busy {
                        "安裝中…"
                    } else {
                        "下載並安裝"
                    })
                    .min_size(egui::vec2(120.0, 32.0));
                    if ui.add_enabled(!self.busy, button).clicked() {
                        self.start();
                    }
                    if self.busy {
                        ui.spinner();
                        if let Some(bytes) = self.progress_bytes {
                            ui.label(format!("已下載 {:.1} MB", bytes as f64 / 1_048_576.0));
                        }
                    }
                });

                if self.last_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, &self.status);
                } else {
                    ui.label(&self.status);
                }

                if !self.log.is_empty() {
                    egui::CollapsingHeader::new("安裝紀錄")
                        .default_open(self.last_error)
                        .show(ui, |ui| {
                            let mut log = self.log.clone();
                            ui.add_sized(
                                [ui.available_width(), 130.0],
                                egui::TextEdit::multiline(&mut log)
                                    .font(egui::TextStyle::Monospace)
                                    .desired_width(f32::INFINITY)
                                    .interactive(false),
                            );
                        });
                }
            });
    }

    fn open_folder(&self) {
        let dir = self.target_dir.trim();
        if dir.is_empty() {
            return;
        }
        let mut command = Command::new("explorer.exe");
        command.arg(dir);
        hide_console(&mut command);
        let _ = command.spawn();
    }
}

fn install(
    repo: &str,
    tag: &str,
    target_dir: &Path,
    sender: &Sender<InstallEvent>,
    note: &mut impl FnMut(String),
) -> Result<(PathBuf, String), String> {
    note(format!(
        "repo: {repo}；版本: {}",
        if tag.is_empty() {
            "最新正式版"
        } else {
            tag
        }
    ));

    let (release_tag, urls) = fetch_release(repo, tag, note)?;
    if release_tag.is_empty() {
        note("GitHub 未回報 tag 名稱，仍嘗試下載所屬 asset。".to_owned());
    } else {
        note(format!("找到 release tag：{release_tag}"));
    }
    let asset_url = pick_windows_asset(&urls).ok_or_else(|| {
        let available = urls.join("、");
        format!("此 release 沒有 Windows zip asset。可用的下載連結：{available}")
    })?;
    note(format!("選用 asset：{}", asset_url));

    let temp_dir =
        std::env::temp_dir().join(format!("oneechk-core-install-{}", std::process::id()));
    if temp_dir.exists() {
        let _ = fs::remove_dir_all(&temp_dir);
    }
    fs::create_dir_all(&temp_dir)
        .map_err(|error| format!("無法建立暫存資料夾 {}：{error}", temp_dir.display()))?;
    let zip_path = temp_dir.join("oneechk-windows.zip");

    note("開始下載（curl.exe；失敗時改用 PowerShell）…".to_owned());
    download(asset_url, &zip_path, sender)?;
    let bytes = fs::metadata(&zip_path).map_or(0, |meta| meta.len());
    note(format!("下載完成：{:.1} MB", bytes as f64 / 1_048_576.0));

    let extract_dir = temp_dir.join("extract");
    let tool = extract_zip(&zip_path, &extract_dir)?;
    note(format!("解壓縮完成（{tool}）。"));

    let found =
        find_oneechk_exe(&extract_dir).ok_or_else(|| "解壓縮後找不到 oneechk.exe。".to_owned())?;
    note(format!("找到執行檔：{}", found.display()));

    fs::create_dir_all(target_dir)
        .map_err(|error| format!("無法建立安裝資料夾 {}：{error}", target_dir.display()))?;
    let installed = target_dir.join("oneechk.exe");
    replace_file(&found, &installed)?;
    note(format!("已安裝：{}", installed.display()));

    let version = probe_version(&installed);
    if !version.is_empty() {
        note(format!("版本檢查：{version}"));
    }

    let _ = fs::remove_dir_all(&temp_dir);
    Ok((installed, version))
}

/// 先複製到同資料夾的暫存檔再改名取代，避免複製中斷留下半個 oneechk.exe。
fn replace_file(source: &Path, installed: &Path) -> Result<(), String> {
    let staged = installed.with_extension("exe.new");
    let _ = fs::remove_file(&staged);
    fs::copy(source, &staged).map_err(|error| {
        format!(
            "無法寫入 {}：{error}。請確認資料夾可寫入且磁碟空間足夠。",
            staged.display()
        )
    })?;

    match fs::rename(&staged, installed) {
        Ok(()) => Ok(()),
        Err(rename_error) => {
            // 少數情況下改名會被檔案鎖或防毒軟體阻擋，退回直接複製一次。
            let direct = fs::copy(&staged, installed);
            let _ = fs::remove_file(&staged);
            direct.map(|_| ()).map_err(|error| {
                format!(
                    "無法寫入 {}：{error}（改名失敗：{rename_error}）。若 oneechk.exe 正在執行，請先關閉再試。",
                    installed.display()
                )
            })
        }
    }
}

fn fetch_release(
    repo: &str,
    tag: &str,
    note: &mut impl FnMut(String),
) -> Result<(String, Vec<String>), String> {
    if tag.is_empty() {
        let url = format!("https://api.github.com/repos/{repo}/releases/latest");
        note(format!("查詢最新 release：{url}"));
        let json = http_get(&url)?;
        let release_tag = json_string_after(&json, "tag_name").unwrap_or_default();
        let urls = json_strings_after(&json, "browser_download_url");
        if urls.is_empty() {
            return Err(format!(
                "GitHub 回應找不到下載連結；請確認 repo 名稱（{repo}）與網路狀態。"
            ));
        }
        return Ok((release_tag, urls));
    }

    let url = format!("https://api.github.com/repos/{repo}/releases/tags/{tag}");
    note(format!("查詢指定 tag：{url}"));
    match http_get(&url) {
        Ok(json) => {
            let release_tag =
                json_string_after(&json, "tag_name").unwrap_or_else(|| tag.to_owned());
            let urls = json_strings_after(&json, "browser_download_url");
            if urls.is_empty() {
                return Err(format!("release {tag} 沒有可下載的 asset。"));
            }
            Ok((release_tag, urls))
        }
        Err(error) => {
            note(format!(
                "GitHub API 查詢失敗（{error}），改嘗試以 tag 推測下載網址。"
            ));
            let version = tag
                .trim_start_matches('v')
                .split('_')
                .next()
                .unwrap_or_default()
                .trim();
            if version.is_empty() {
                return Err(format!("查詢 release 失敗：{error}"));
            }
            let direct = format!(
                "https://github.com/{repo}/releases/download/{tag}/oneechk-{version}{WINDOWS_ASSET_SUFFIX}"
            );
            note(format!("推測下載網址：{direct}"));
            Ok((tag.to_owned(), vec![direct]))
        }
    }
}

fn pick_windows_asset(urls: &[String]) -> Option<&str> {
    let windows_zip = |url: &&String| {
        let lower = url.to_ascii_lowercase();
        lower.ends_with(".zip") && lower.contains("windows")
    };
    urls.iter()
        .find(|url| windows_zip(url) && url.to_ascii_lowercase().contains("x86_64-pc-windows-gnu"))
        .or_else(|| urls.iter().find(windows_zip))
        .map(String::as_str)
}

fn http_get(url: &str) -> Result<String, String> {
    let mut command = Command::new("curl.exe");
    command.args([
        "-sSL",
        "--fail",
        "--connect-timeout",
        "15",
        "--max-time",
        "90",
        "-H",
        "Accept: application/vnd.github+json",
        "-H",
        "User-Agent: oneechk-ui",
        url,
    ]);
    hide_console(&mut command);
    let curl_error = match command.output() {
        Ok(output) if output.status.success() => {
            return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
        }
        Ok(output) => format!("curl 退出碼 {:?}", output.status.code()),
        Err(error) => format!("無法執行 curl.exe：{error}"),
    };

    let script = format!(
        "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; (Invoke-WebRequest -UseBasicParsing -Uri '{}' -Headers @{{'User-Agent'='oneechk-ui';'Accept'='application/vnd.github+json'}}).Content",
        ps_quote(url)
    );
    let mut powershell = Command::new("powershell.exe");
    powershell.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
    hide_console(&mut powershell);
    match powershell.output() {
        Ok(output) if output.status.success() => {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        }
        Ok(output) => Err(format!(
            "{curl_error}；PowerShell 備援亦失敗：{}",
            first_line(&String::from_utf8_lossy(&output.stderr))
        )),
        Err(error) => Err(format!("{curl_error}；也無法執行 powershell.exe：{error}")),
    }
}

fn download(url: &str, dest: &Path, sender: &Sender<InstallEvent>) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("無法建立暫存資料夾 {}：{error}", parent.display()))?;
    }
    match curl_download(url, dest, sender) {
        Ok(()) if dest.is_file() => Ok(()),
        Ok(()) => Err("下載結束但找不到暫存檔案。".to_owned()),
        Err(curl_error) => {
            let _ = sender.send(InstallEvent::Log(format!(
                "curl 下載失敗（{curl_error}），改用 PowerShell 下載…"
            )));
            let script = format!(
                "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; Invoke-WebRequest -UseBasicParsing -Uri '{}' -OutFile '{}'",
                ps_quote(url),
                ps_quote(&dest.display().to_string())
            );
            let mut powershell = Command::new("powershell.exe");
            powershell.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
            hide_console(&mut powershell);
            match powershell.output() {
                Ok(output) if output.status.success() && dest.is_file() => Ok(()),
                Ok(output) => Err(format!(
                    "{curl_error}；PowerShell 備援亦失敗：{}",
                    first_line(&String::from_utf8_lossy(&output.stderr))
                )),
                Err(error) => Err(format!("{curl_error}；也無法執行 powershell.exe：{error}")),
            }
        }
    }
}

fn curl_download(url: &str, dest: &Path, sender: &Sender<InstallEvent>) -> Result<(), String> {
    let mut command = Command::new("curl.exe");
    command
        .args([
            "-sSL",
            "--fail",
            "--retry",
            "2",
            "--connect-timeout",
            "15",
            "-o",
        ])
        .arg(dest)
        .arg(url);
    hide_console(&mut command);

    let stop = Arc::new(AtomicBool::new(false));
    let monitor = {
        let stop = Arc::clone(&stop);
        let dest = dest.to_path_buf();
        let sender = sender.clone();
        thread::spawn(move || {
            let mut last = 0_u64;
            while !stop.load(Ordering::Relaxed) {
                if let Ok(meta) = fs::metadata(&dest) {
                    let len = meta.len();
                    if len > last {
                        last = len;
                        let _ = sender.send(InstallEvent::Progress(len));
                    }
                }
                thread::sleep(Duration::from_millis(200));
            }
        })
    };

    let output = command.output();
    stop.store(true, Ordering::Relaxed);
    let _ = monitor.join();

    match output {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(format!(
            "curl 退出碼 {:?}：{}",
            output.status.code(),
            first_line(&String::from_utf8_lossy(&output.stderr))
        )),
        Err(error) => Err(format!("無法執行 curl.exe：{error}")),
    }
}

fn extract_zip(zip: &Path, dest: &Path) -> Result<String, String> {
    fs::create_dir_all(dest)
        .map_err(|error| format!("無法建立解壓縮資料夾 {}：{error}", dest.display()))?;

    let mut tar = Command::new("tar.exe");
    tar.arg("-xf").arg(zip).arg("-C").arg(dest);
    hide_console(&mut tar);
    if let Ok(output) = tar.output() {
        if output.status.success() {
            return Ok("tar.exe".to_owned());
        }
    }

    let script = format!(
        "Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
        ps_quote(&zip.display().to_string()),
        ps_quote(&dest.display().to_string())
    );
    let mut powershell = Command::new("powershell.exe");
    powershell.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
    hide_console(&mut powershell);
    match powershell.output() {
        Ok(output) if output.status.success() => Ok("PowerShell Expand-Archive".to_owned()),
        Ok(output) => Err(format!(
            "解壓縮失敗：tar.exe 與 PowerShell 都無法處理 {}；{}",
            zip.display(),
            first_line(&String::from_utf8_lossy(&output.stderr))
        )),
        Err(error) => Err(format!("無法執行解壓縮工具：{error}")),
    }
}

fn find_oneechk_exe(root: &Path) -> Option<PathBuf> {
    let mut dirs = vec![root.to_path_buf()];
    for _ in 0..4 {
        let mut next = Vec::new();
        for dir in dirs {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    next.push(path);
                } else if path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("oneechk.exe"))
                {
                    return Some(path);
                }
            }
        }
        dirs = next;
        if dirs.is_empty() {
            break;
        }
    }
    None
}

fn probe_version(exe: &Path) -> String {
    let mut command = Command::new(exe);
    command.arg("--version");
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    hide_console(&mut command);
    let Ok(mut child) = command.spawn() else {
        return String::new();
    };

    // 損壞或等待輸入的執行檔不該讓背景工作卡死：最多等 10 秒就強制結束。
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return String::new();
            }
            Err(_) => return String::new(),
        }
    }

    let mut text = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_string(&mut text);
    }
    if let Some(mut stderr) = child.stderr.take() {
        let _ = stderr.read_to_string(&mut text);
    }
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .to_owned()
}

/// PowerShell 單引號字串內的單引號需寫成兩個單引號。
fn ps_quote(text: &str) -> String {
    text.replace('\'', "''")
}

fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("沒有錯誤訊息")
        .to_owned()
}

fn json_string_after(json: &str, key: &str) -> Option<String> {
    json_strings_after(json, key).into_iter().next()
}

/// 取出所有指定 key 的字串值（只掃描目標 key，不需完整 JSON 解析器）。
fn json_strings_after(json: &str, key: &str) -> Vec<String> {
    let needle = format!("\"{key}\"");
    let bytes = json.as_bytes();
    let mut values = Vec::new();
    let mut search_from = 0;
    while let Some(offset) = json[search_from..].find(&needle) {
        let key_end = search_from + offset + needle.len();
        let mut index = key_end;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index < bytes.len() && bytes[index] == b':' {
            index += 1;
        } else {
            search_from = key_end;
            continue;
        }
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if let Some((value, consumed)) = read_json_string(&json[index..]) {
            values.push(value);
            search_from = index + consumed;
        } else {
            search_from = key_end;
        }
    }
    values
}

/// 從 JSON 字串開頭讀出一個字串值，回傳（解碼後字串, 消耗的位元組數）。
fn read_json_string(text: &str) -> Option<(String, usize)> {
    let mut chars = text.char_indices();
    match chars.next() {
        Some((_, '"')) => {}
        _ => return None,
    }

    let mut value = String::new();
    while let Some((index, ch)) = chars.next() {
        match ch {
            '"' => return Some((value, index + 1)),
            '\\' => {
                let (_, escape) = chars.next()?;
                match escape {
                    'n' => value.push('\n'),
                    't' => value.push('\t'),
                    'r' => value.push('\r'),
                    '"' => value.push('"'),
                    '\\' => value.push('\\'),
                    '/' => value.push('/'),
                    'u' => {
                        let mut hex = String::new();
                        for _ in 0..4 {
                            hex.push(chars.next()?.1);
                        }
                        let decoded = u32::from_str_radix(&hex, 16)
                            .ok()
                            .and_then(char::from_u32)
                            .unwrap_or('\u{fffd}');
                        value.push(decoded);
                    }
                    other => value.push(other),
                }
            }
            other => value.push(other),
        }
    }
    None
}

#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    let _ = command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_windows_asset_and_skips_linux() {
        let urls = vec![
            "https://example.com/oneechk-2.0.2-x86_64-unknown-linux-musl.tar.gz".to_owned(),
            "https://example.com/oneechk-2.0.2-x86_64-pc-windows-gnu.zip".to_owned(),
        ];
        assert_eq!(
            pick_windows_asset(&urls),
            Some("https://example.com/oneechk-2.0.2-x86_64-pc-windows-gnu.zip")
        );
    }

    #[test]
    fn no_windows_asset_returns_none() {
        let urls = vec!["https://example.com/oneechk-linux.tar.gz".to_owned()];
        assert_eq!(pick_windows_asset(&urls), None);
    }

    #[test]
    fn parses_release_json() {
        let json = r#"{
            "tag_name": "v2.0.2_Auto",
            "assets": [
                {"name": "oneechk-2.0.2-x86_64-pc-windows-gnu.zip", "browser_download_url": "https://github.com/x/y/releases/download/v2.0.2_Auto/oneechk-2.0.2-x86_64-pc-windows-gnu.zip"},
                {"name": "oneechk-2.0.2-x86_64-unknown-linux-musl.tar.gz", "browser_download_url": "https://github.com/x/y/releases/download/v2.0.2_Auto/oneechk-2.0.2-x86_64-unknown-linux-musl.tar.gz"}
            ]
        }"#;
        assert_eq!(
            json_string_after(json, "tag_name").as_deref(),
            Some("v2.0.2_Auto")
        );
        let urls = json_strings_after(json, "browser_download_url");
        assert_eq!(urls.len(), 2);
        assert!(urls[0].ends_with("oneechk-2.0.2-x86_64-pc-windows-gnu.zip"));
    }

    #[test]
    fn json_extractor_handles_escapes_and_missing_keys() {
        let json = r#"{"tag_name":"v1.0.0","html_url":"https:\/\/github.com\/x\/y"}"#;
        assert_eq!(
            json_string_after(json, "tag_name").as_deref(),
            Some("v1.0.0")
        );
        assert_eq!(
            json_string_after(json, "html_url").as_deref(),
            Some("https://github.com/x/y")
        );
        assert_eq!(json_string_after(json, "browser_download_url"), None);
        assert!(json_strings_after(json, "browser_download_url").is_empty());
    }

    #[test]
    fn reads_unicode_escapes() {
        assert_eq!(
            read_json_string(r#""a\u4e2db""#),
            Some(("a中b".to_owned(), 10))
        );
    }

    #[test]
    fn http_get_reports_failure_for_unreachable_host() {
        assert!(http_get("http://127.0.0.1:1/").is_err());
    }

    #[test]
    fn ps_quote_doubles_single_quotes() {
        assert_eq!(
            ps_quote(r"C:\Users\O'Brien\x.exe"),
            "C:\\Users\\O''Brien\\x.exe"
        );
        assert_eq!(ps_quote("plain"), "plain");
    }

    #[test]
    fn replace_file_overwrites_existing_without_leaking_staging() {
        let root = std::env::temp_dir().join(format!("oneechk-ui-replace-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let source = root.join("new.exe");
        fs::write(&source, b"NEW").unwrap();
        let installed = root.join("oneechk.exe");
        fs::write(&installed, b"OLD").unwrap();

        replace_file(&source, &installed).unwrap();
        assert_eq!(fs::read(&installed).unwrap(), b"NEW");
        assert!(!root.join("oneechk.exe.new").exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn probe_version_missing_exe_returns_empty() {
        assert!(probe_version(Path::new(r"Z:\definitely-missing-oneechk.exe")).is_empty());
    }

    #[test]
    fn extracts_windows_zip_and_finds_core() {
        let has_tar = Command::new("tar.exe")
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success());
        if !has_tar {
            eprintln!("略過：此環境沒有 tar.exe");
            return;
        }

        let root = std::env::temp_dir().join(format!("oneechk-ui-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let source = root.join("src");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("oneechk.exe"), b"MZ").unwrap();
        fs::write(source.join("README.md"), b"readme").unwrap();
        let zip = root.join("core.zip");
        let created = Command::new("tar.exe")
            .arg("-a")
            .arg("-cf")
            .arg(&zip)
            .arg("-C")
            .arg(&source)
            .arg(".")
            .status()
            .unwrap();
        assert!(created.success());

        let extracted = root.join("extract");
        extract_zip(&zip, &extracted).expect("解壓縮測試 zip");
        let found = find_oneechk_exe(&extracted).expect("應能找到 oneechk.exe");
        assert!(found.ends_with("oneechk.exe"));

        let _ = fs::remove_dir_all(&root);
    }
}
