# Onee Checker 桌面工具

獨立的 Windows 桌面 UI，自 `onee_checker` 本體拆分為獨立 git 儲存庫，透過參數陣列直接啟動使用者指定的 `oneechk.exe`。UI 不載入 hash library，也不經由 shell 組合命令；`oneechk.exe` 可單獨更新或替換。

## 功能

- 產生校驗碼：支援所有 oneechk 演算法、多選演算法、自訂可變輸出長度、buffer、執行緒與輸出檔案。
- 驗證校驗碼：可自動依副檔名推斷算法，也可手動選擇算法與根目錄。
- JSON 目錄清單與文字目錄樹：可選輸出位置、大小格式與時間欄位。
- 使用 Windows 原生檔案/資料夾對話框選取工具、輸入路徑、輸出檔與驗證根目錄；也可把檔案或資料夾拖放到視窗，自動帶入輸入路徑。
- 內建「從 GitHub 安裝核心」面板：可指定 repo、版本 tag 與安裝資料夾，直接下載 oneechk 官方 Release 的 Windows zip、解壓縮並安裝 `oneechk.exe`，完成後自動填入執行檔路徑並顯示版本。
- 下載與解壓縮優先使用 Windows 內建 `curl.exe`／`tar.exe`，失敗時改用 PowerShell 的 `Invoke-WebRequest`／`Expand-Archive` 備援，沒有額外執行期依賴。
- 內嵌 Noto Sans TC 繁體中文字型，在未安裝中文語系字型的 Windows 上也能正常顯示。
- 子程序在背景執行，UI 不會因大型目錄計算而凍結。
- 分開顯示 stdout、stderr、命令參數與退出碼，適用於確認自動化結果。
- 預設使用 `--quiet`，不把終端進度列混入輸出；所有輸入都以獨立參數傳遞，不經 shell 解讀。

## 執行需求

- Windows 10 或更新版本。
- 建置時需要 Rust stable 與網路連線下載 eframe/rfd 相依套件。
- 執行時不需安裝 Rust 或 .NET；Release 壓縮檔內含 `oneechk-ui.exe`，另需一個 `oneechk.exe` 核心（可用內建「從 GitHub 安裝核心」下載，或按「選擇程式…」指定其他位置的核心）。
- GitHub 安裝功能需要網路連線；離線時仍可手動指定本機的 `oneechk.exe`。
- 隨附字型採 SIL Open Font License 1.1，授權全文見 `assets/OFL.txt`；打包時會一併複製至 `dist/NOTICE-Noto-Sans-TC.txt`。

## 建置與打包

`build-windows.ps1` 會同時建置 CLI 與 UI，因此兩個專案需放在同一上層目錄：

```text
<workspace>/
├── onee_checker/       # CLI 與 library
└── onee_checker_ui/    # 本專案
```

在此資料夾執行：

```powershell
.\build-windows.ps1
```

完成後，`dist\` 會包含 UI、CLI 及字型授權檔。將整個 `dist` 資料夾複製到 Windows 電腦即可使用。

若只想建置 UI：

```powershell
cargo build --release --manifest-path .\Cargo.toml
```

## 自動建置與發布（GitHub Actions）

推送 `v*` 標籤後，`.github/workflows/release.yml` 會自動在 `windows-latest` 上執行格式檢查、Clippy、測試、`cargo build --release --locked`，打包成 `oneechk-ui-<版本>-x86_64-pc-windows-msvc.zip`，並建立 GitHub Release 上傳壓縮檔。手動觸發可在 Actions 頁面使用 `workflow_dispatch`。

發布新版本：

```powershell
# 1. 更新 Cargo.toml 的 version（Workflow 會驗證標籤與版本一致）
# 2. 提交並推送 main
git add -A
git commit -m "chore: 發佈 v0.2.0"
git push origin main
# 3. 建立並推送標籤，觸發發布
git tag v0.2.0
git push origin v0.2.0
```

`.github/workflows/ci.yml` 會在推送 `main` 與 Pull Request 時執行相同的格式、Clippy 與測試檢查。

## 使用方式

1. 開啟 `oneechk-ui.exe`。若同一資料夾有 `oneechk.exe`，程式會自動選用；否則按「選擇程式…」指定，或使用「從 GitHub 安裝核心」。
2. 選擇「產生校驗碼」、「驗證校驗碼」、「JSON 目錄清單」或「文字目錄樹」。
3. 選取輸入檔案/目錄、設定選項，按「執行」。
4. 檢視 stdout、stderr 和退出碼。退出碼沿用 oneechk 契約：`0` 成功、`1` hash 不匹配、`2` I/O/執行錯誤、`3` 參數或格式錯誤。

## 從 GitHub 安裝核心

「從 GitHub 安裝核心」面板預設 repo 為 `XiaoYao-Www/onee_checker`，版本 tag 留空代表最新正式版；也可輸入例如 `v2.0.3_Auto`。安裝資料夾預設為 UI 執行檔所在資料夾。按下「下載並安裝」後會：

1. 以 GitHub API 查詢 release（指定 tag 查詢失敗時，會改以 tag 推測 Release 下載網址）。
2. 下載名稱含 `windows` 與 `x86_64-pc-windows-gnu` 的 zip，並顯示下載進度。
3. 解壓縮、找出 `oneechk.exe` 並複製到安裝資料夾。
4. 執行 `oneechk --version` 確認版本，並自動填入 `oneechk.exe` 路徑。

安裝紀錄會保留在面板中；若 `oneechk.exe` 正在執行，請先關閉再安裝，以免檔案被鎖定。

## 專案結構

```text
onee_checker_ui/
├── Cargo.toml          # 獨立 Cargo workspace
├── build-windows.ps1   # 建置並將 CLI 與 UI 打包到 dist/
├── assets/             # 內嵌中文字型與字型授權
├── .github/workflows/  # CI 與 Release 自動化
└── src/
    ├── main.rs         # Windows 桌面介面與子程序調用
    └── install.rs      # GitHub Releases 下載與安裝邏輯
```

## 授權

本專案採 MIT 授權，全文見 `LICENSE`。內嵌的 Noto Sans TC 字型另採 SIL Open Font License 1.1，全文見 `assets/OFL.txt`。

