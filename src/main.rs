#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::ffi::OsString;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use eframe::egui;
use rfd::FileDialog;

const ALGORITHMS: &[&str] = &[
    "md5",
    "sha1",
    "sha224",
    "sha256",
    "sha384",
    "sha512",
    "sha3-256",
    "sha3-512",
    "shake128",
    "shake256",
    "blake2s256",
    "blake2b512",
    "blake3",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CommandKind {
    Hash,
    Verify,
    Json,
    Txt,
}

impl CommandKind {
    const ALL: [Self; 4] = [Self::Hash, Self::Verify, Self::Json, Self::Txt];

    const fn label(self) -> &'static str {
        match self {
            Self::Hash => "產生校驗碼",
            Self::Verify => "驗證校驗碼",
            Self::Json => "JSON 目錄清單",
            Self::Txt => "文字目錄樹",
        }
    }

    const fn cli_name(self) -> &'static str {
        match self {
            Self::Hash => "hash",
            Self::Verify => "verify",
            Self::Json => "json",
            Self::Txt => "txt",
        }
    }
}

struct ExecutionResult {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    error: Option<String>,
}

struct OneeUi {
    exe_path: String,
    command: CommandKind,
    input_path: String,
    output_path: String,
    root_path: String,
    algorithm: String,
    selected_algorithms: Vec<String>,
    length: String,
    buffer: String,
    threads: String,
    tree_size: String,
    show_modified: bool,
    show_created: bool,
    is_running: bool,
    receiver: Option<Receiver<ExecutionResult>>,
    stdout: String,
    stderr: String,
    exit_code: Option<i32>,
    status: String,
    validation_error: Option<String>,
    command_preview: String,
}

impl Default for OneeUi {
    fn default() -> Self {
        let sibling_exe = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|parent| parent.join("oneechk.exe")))
            .filter(|path| path.is_file())
            .map_or_else(|| "oneechk.exe".to_owned(), |path| path.display().to_string());

        Self {
            exe_path: sibling_exe,
            command: CommandKind::Hash,
            input_path: String::new(),
            output_path: String::new(),
            root_path: String::new(),
            algorithm: "自動判斷（依副檔名）".to_owned(),
            selected_algorithms: vec!["sha256".to_owned()],
            length: String::new(),
            buffer: "1M".to_owned(),
            threads: String::new(),
            tree_size: "不顯示".to_owned(),
            show_modified: false,
            show_created: false,
            is_running: false,
            receiver: None,
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            status: "選擇命令並指定輸入路徑。".to_owned(),
            validation_error: None,
            command_preview: String::new(),
        }
    }
}

impl OneeUi {
    fn pick_executable(&mut self) {
        if let Some(path) = FileDialog::new().add_filter("Windows 應用程式", &["exe"]).pick_file()
        {
            self.exe_path = path.display().to_string();
        }
    }

    fn pick_input_file(&mut self) {
        let dialog = FileDialog::new();
        let path = if self.command == CommandKind::Verify {
            let mut extensions = ALGORITHMS.to_vec();
            extensions.push("txt");
            dialog.add_filter("校驗碼清單", &extensions).pick_file()
        } else {
            dialog.pick_file()
        };
        if let Some(path) = path {
            self.input_path = path.display().to_string();
        }
    }

    fn pick_input_folder(&mut self) {
        if let Some(path) = FileDialog::new().pick_folder() {
            self.input_path = path.display().to_string();
        }
    }

    fn pick_root_folder(&mut self) {
        if let Some(path) = FileDialog::new().pick_folder() {
            self.root_path = path.display().to_string();
        }
    }

    fn pick_output(&mut self) {
        let (name, extension) = match self.command {
            CommandKind::Hash => {
                let algorithm = self.selected_algorithms.first().map_or("sha256", String::as_str);
                (format!("checksums.{algorithm}"), algorithm.to_owned())
            }
            CommandKind::Verify => (String::new(), "txt".to_owned()),
            CommandKind::Json => ("structure.json".to_owned(), "json".to_owned()),
            CommandKind::Txt => ("tree.txt".to_owned(), "txt".to_owned()),
        };
        let mut dialog = FileDialog::new();
        if !extension.is_empty() {
            dialog = dialog.add_filter("輸出檔案", &[extension.as_str()]);
        }
        if !name.is_empty() {
            dialog = dialog.set_file_name(&name);
        }
        if let Some(path) = dialog.save_file() {
            self.output_path = path.display().to_string();
        }
    }

    fn validate_and_build_args(&self) -> Result<Vec<OsString>, String> {
        if self.exe_path.trim().is_empty() {
            return Err("請先指定 oneechk.exe。".to_owned());
        }
        if self.input_path.trim().is_empty() {
            return Err("請先選擇輸入檔案或目錄。".to_owned());
        }

        let mut args = vec![OsString::from(self.command.cli_name())];
        args.push(OsString::from(self.input_path.trim()));

        match self.command {
            CommandKind::Hash => {
                if self.selected_algorithms.is_empty() {
                    return Err("至少選擇一種雜湊演算法。".to_owned());
                }
                if !self.output_path.trim().is_empty() && self.selected_algorithms.len() > 1 {
                    return Err("指定單一輸出檔時只能選擇一種演算法；多種演算法請清除輸出檔欄位，讓 oneechk 分別產生清單。".to_owned());
                }
                if !self.length.trim().is_empty() {
                    if !self.length.trim().parse::<u16>().is_ok_and(|value| value > 0) {
                        return Err("輸出長度必須是 1 到 65535 之間的整數 bytes。".to_owned());
                    }
                    if self
                        .selected_algorithms
                        .iter()
                        .any(|algo| !matches!(algo.as_str(), "shake128" | "shake256" | "blake3"))
                    {
                        return Err("自訂長度只適用 SHAKE-128、SHAKE-256 或 BLAKE3。".to_owned());
                    }
                }
                for algo in &self.selected_algorithms {
                    push_pair(&mut args, "--algo", algo);
                }
                if !self.length.trim().is_empty() {
                    push_pair(&mut args, "--length", self.length.trim());
                }
                self.append_common_options(&mut args);
                if !self.output_path.trim().is_empty() {
                    push_pair(&mut args, "--output", self.output_path.trim());
                }
            }
            CommandKind::Verify => {
                if self.algorithm != "自動判斷（依副檔名）" {
                    push_pair(&mut args, "--algo", &self.algorithm);
                }
                if !self.root_path.trim().is_empty() {
                    push_pair(&mut args, "--root", self.root_path.trim());
                }
                self.append_common_options(&mut args);
            }
            CommandKind::Json => {
                if !self.output_path.trim().is_empty() {
                    push_pair(&mut args, "--output", self.output_path.trim());
                }
            }
            CommandKind::Txt => {
                if !self.output_path.trim().is_empty() {
                    push_pair(&mut args, "--output", self.output_path.trim());
                }
                match self.tree_size.as_str() {
                    "二進位" => push_pair(&mut args, "--size", "binary"),
                    "十進位" => push_pair(&mut args, "--size", "decimal"),
                    "原始位元組" => push_pair(&mut args, "--size", "raw"),
                    _ => {}
                }
                if self.show_modified {
                    args.push(OsString::from("--modified"));
                }
                if self.show_created {
                    args.push(OsString::from("--created"));
                }
            }
        }

        // GUI uses quiet mode to suppress terminal progress output.
        args.push(OsString::from("--quiet"));
        Ok(args)
    }

    fn append_common_options(&self, args: &mut Vec<OsString>) {
        push_pair(args, "--buffer", self.buffer.trim());
        if !self.threads.trim().is_empty() {
            push_pair(args, "--threads", self.threads.trim());
        }
    }

    fn start(&mut self) {
        self.validation_error = None;
        let args = match self.validate_and_build_args() {
            Ok(args) => args,
            Err(error) => {
                self.validation_error = Some(error);
                return;
            }
        };

        let executable = OsString::from(self.exe_path.trim());
        self.command_preview = format!(
            "{} {}",
            quote_for_display(self.exe_path.trim()),
            args.iter()
                .map(|arg| quote_for_display(&arg.to_string_lossy()))
                .collect::<Vec<_>>()
                .join(" ")
        );
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.is_running = true;
        self.exit_code = None;
        self.stdout.clear();
        self.stderr.clear();
        self.status = "正在執行 oneechk…".to_owned();

        thread::spawn(move || {
            let result = Command::new(executable)
                .args(args)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output();

            let execution = match result {
                Ok(output) => ExecutionResult {
                    exit_code: output.status.code(),
                    stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                    error: None,
                },
                Err(error) => ExecutionResult {
                    exit_code: None,
                    stdout: String::new(),
                    stderr: String::new(),
                    error: Some(format!("無法啟動 oneechk：{error}")),
                },
            };
            let _ = sender.send(execution);
        });
    }

    fn poll_result(&mut self, ctx: &egui::Context) {
        let Some(receiver) = &self.receiver else {
            return;
        };
        match receiver.try_recv() {
            Ok(result) => {
                self.is_running = false;
                self.exit_code = result.exit_code;
                self.stdout = result.stdout;
                self.stderr = result.stderr;
                self.status = match result.exit_code {
                    Some(0) => "完成：oneechk 回傳成功。".to_owned(),
                    Some(1) => "驗證未通過：至少一筆雜湊不匹配。".to_owned(),
                    Some(code) => format!("執行失敗：oneechk 回傳退出碼 {code}。"),
                    None => result.error.unwrap_or_else(|| "oneechk 無法啟動。".to_owned()),
                };
                self.receiver = None;
            }
            Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.is_running = false;
                self.status = "執行工作已中斷，沒有收到結果。".to_owned();
                self.receiver = None;
            }
        }
    }

    fn draw_executable_picker(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("oneechk.exe");
            ui.add(egui::TextEdit::singleline(&mut self.exe_path).desired_width(f32::INFINITY));
            if ui.button("選擇程式…").clicked() {
                self.pick_executable();
            }
        });
        ui.small("支援選取專案編譯的 oneechk.exe，或其他相容版本。");
    }

    fn draw_command_tabs(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for command in CommandKind::ALL {
                ui.selectable_value(&mut self.command, command, command.label());
            }
        });
    }

    fn draw_input_picker(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(if self.command == CommandKind::Verify {
                "校驗碼清單"
            } else {
                "輸入路徑"
            });
            ui.add(
                egui::TextEdit::singleline(&mut self.input_path)
                    .desired_width(f32::INFINITY)
                    .hint_text("選擇檔案或目錄"),
            );
            match self.command {
                CommandKind::Hash => {
                    if ui.button("選擇檔案…").clicked() {
                        self.pick_input_file();
                    }
                    if ui.button("選擇目錄…").clicked() {
                        self.pick_input_folder();
                    }
                }
                CommandKind::Verify => {
                    if ui.button("瀏覽…").clicked() {
                        self.pick_input_file();
                    }
                }
                CommandKind::Json | CommandKind::Txt => {
                    if ui.button("選擇目錄…").clicked() {
                        self.pick_input_folder();
                    }
                }
            }
        });
    }

    fn draw_output_picker(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("輸出檔案");
            ui.add(
                egui::TextEdit::singleline(&mut self.output_path)
                    .desired_width(f32::INFINITY)
                    .hint_text("留空使用 oneechk 預設檔名"),
            );
            if ui.button("另存新檔…").clicked() {
                self.pick_output();
            }
        });
    }

    fn draw_hash_options(&mut self, ui: &mut egui::Ui) {
        ui.label("雜湊演算法（可複選；留一項即可產生單一校驗碼檔）");
        egui::Grid::new("algorithm_grid").num_columns(4).show(ui, |ui| {
            for (index, algorithm) in ALGORITHMS.iter().enumerate() {
                let mut selected = self.selected_algorithms.iter().any(|item| item == algorithm);
                if ui.checkbox(&mut selected, *algorithm).changed() {
                    if selected {
                        self.selected_algorithms.push((*algorithm).to_owned());
                    } else {
                        self.selected_algorithms.retain(|item| item != algorithm);
                    }
                }
                if (index + 1) % 4 == 0 {
                    ui.end_row();
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("可變長度（bytes）");
            ui.add(
                egui::TextEdit::singleline(&mut self.length)
                    .desired_width(140.0)
                    .hint_text("留空使用預設長度"),
            );
            ui.label("僅 SHAKE 與 BLAKE3 支援");
        });
        self.draw_common_options(ui);
    }

    fn draw_common_options(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("讀取緩衝區");
            ui.add(egui::TextEdit::singleline(&mut self.buffer).desired_width(100.0));
            ui.label("例如 1M、4K、512B");
            ui.add_space(16.0);
            ui.label("執行緒數");
            ui.add(
                egui::TextEdit::singleline(&mut self.threads).desired_width(80.0).hint_text("自動"),
            );
        });
    }

    fn draw_verify_options(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("雜湊演算法");
            egui::ComboBox::from_id_salt("verify_algorithm")
                .selected_text(&self.algorithm)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.algorithm,
                        "自動判斷（依副檔名）".to_owned(),
                        "自動判斷（依副檔名）",
                    );
                    for algorithm in ALGORITHMS {
                        ui.selectable_value(
                            &mut self.algorithm,
                            (*algorithm).to_owned(),
                            *algorithm,
                        );
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label("驗證根目錄");
            ui.add(
                egui::TextEdit::singleline(&mut self.root_path)
                    .desired_width(f32::INFINITY)
                    .hint_text("留空使用校驗碼檔所在目錄"),
            );
            if ui.button("選擇目錄…").clicked() {
                self.pick_root_folder();
            }
        });
        self.draw_common_options(ui);
    }

    fn draw_txt_options(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("大小格式");
            egui::ComboBox::from_id_salt("tree_size").selected_text(&self.tree_size).show_ui(
                ui,
                |ui| {
                    for value in ["不顯示", "二進位", "十進位", "原始位元組"] {
                        ui.selectable_value(&mut self.tree_size, value.to_owned(), value);
                    }
                },
            );
            ui.checkbox(&mut self.show_modified, "顯示修改時間");
            ui.checkbox(&mut self.show_created, "顯示建立時間");
        });
    }

    fn draw_run_button(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let button = egui::Button::new(if self.is_running { "執行中…" } else { "執行" })
                .min_size(egui::vec2(120.0, 38.0));
            if ui.add_enabled(!self.is_running, button).clicked() {
                self.start();
            }
            if self.is_running {
                ui.spinner();
            }
            ui.label("執行期間視窗仍可使用；完成後顯示原始輸出與退出碼。");
        });
        if let Some(error) = &self.validation_error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
    }

    fn draw_results(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.horizontal(|ui| {
            ui.heading("執行結果");
            if let Some(code) = self.exit_code {
                let color = match code {
                    0 => egui::Color32::LIGHT_GREEN,
                    1 => egui::Color32::YELLOW,
                    _ => egui::Color32::LIGHT_RED,
                };
                ui.colored_label(color, format!("Exit code: {code}"));
            }
        });
        ui.label(&self.status);
        if !self.command_preview.is_empty() {
            ui.collapsing("查看實際呼叫參數", |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.command_preview)
                        .desired_width(f32::INFINITY)
                        .interactive(false),
                );
            });
        }
        egui::CollapsingHeader::new("標準輸出 stdout").default_open(true).show(ui, |ui| {
            ui.add_sized(
                [ui.available_width(), 150.0],
                egui::TextEdit::multiline(&mut self.stdout)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY),
            );
        });
        egui::CollapsingHeader::new("錯誤與狀態 stderr").default_open(true).show(ui, |ui| {
            ui.add_sized(
                [ui.available_width(), 110.0],
                egui::TextEdit::multiline(&mut self.stderr)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY),
            );
        });
    }
}

impl eframe::App for OneeUi {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_result(&ctx);
        ui.horizontal(|ui| {
            ui.heading("Onee Checker 桌面工具");
            ui.label("使用圖形介面執行 oneechk.exe");
        });
        ui.separator();

        egui::ScrollArea::vertical().show(ui, |ui| {
            self.draw_executable_picker(ui);
            ui.separator();
            self.draw_command_tabs(ui);
            ui.separator();

            match self.command {
                CommandKind::Hash => {
                    self.draw_input_picker(ui);
                    self.draw_output_picker(ui);
                    self.draw_hash_options(ui);
                }
                CommandKind::Verify => {
                    self.draw_input_picker(ui);
                    self.draw_verify_options(ui);
                }
                CommandKind::Json => {
                    self.draw_input_picker(ui);
                    self.draw_output_picker(ui);
                }
                CommandKind::Txt => {
                    self.draw_input_picker(ui);
                    self.draw_output_picker(ui);
                    self.draw_txt_options(ui);
                }
            }

            ui.separator();
            self.draw_run_button(ui);
            self.draw_results(ui);
        });

        if self.is_running {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

fn push_pair(args: &mut Vec<OsString>, name: &str, value: &str) {
    args.push(OsString::from(name));
    args.push(OsString::from(value));
}

fn quote_for_display(value: &str) -> String {
    format!("{value:?}")
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1080.0, 820.0])
            .with_min_inner_size([860.0, 650.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "Onee Checker 桌面工具",
        options,
        Box::new(|creation_context| {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                "Noto Sans TC".to_owned(),
                std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
                    "../assets/NotoSansTC-VF.ttf"
                ))),
            );
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "Noto Sans TC".to_owned());
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .push("Noto Sans TC".to_owned());
            creation_context.egui_ctx.set_fonts(fonts);
            Ok(Box::new(OneeUi::default()))
        }),
    )
}
