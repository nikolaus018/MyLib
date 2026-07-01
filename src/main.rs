#![windows_subsystem = "windows"]

use std::process::Command;
use uuid::Uuid;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};
use wry::WebViewBuilder;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::os::windows::process::CommandExt;
use winit::platform::windows::{WindowAttributesExtWindows, CornerPreference};

use directories::ProjectDirs;
use std::fs;
use std::path::PathBuf;
use std::net::{TcpListener, TcpStream};
use std::io::{Read, Write};
use std::thread;
use sysinfo::System;

pub enum AppEvent {
    Ipc(String),
    DispatchToWebView(String),
    PollGames,
    GamepadInput(String),
    CloseWindow,
    ToggleWindow,
}

fn get_base_games_dir() -> PathBuf {
    match ProjectDirs::from("com", "MyLib", "MyLib") {
        Some(proj_dirs) => proj_dirs.data_local_dir().join("games"),
        None => PathBuf::from("mylib-data").join("games"),
    }
}

fn start_http_server() -> Option<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").ok()?;
    let port = listener.local_addr().ok()?.port();
    
    let base_dir = get_base_games_dir();
    
    thread::spawn(move || {
        for stream in listener.incoming() {
            if let Ok(mut stream) = stream {
                let base_dir_clone = base_dir.clone();
                thread::spawn(move || {
                    let mut buffer = [0; 4096];
                    if let Ok(size) = stream.read(&mut buffer) {
                        let request_str = String::from_utf8_lossy(&buffer[..size]);
                        let mut lines = request_str.lines();
                        if let Some(req_line) = lines.next() {
                            let parts: Vec<&str> = req_line.split_whitespace().collect();
                            if parts.len() >= 2 && parts[0] == "GET" {
                                let path = parts[1];
                                handle_http_get(&mut stream, path, &base_dir_clone);
                            }
                        }
                    }
                });
            }
        }
    });
    
    Some(port)
}

fn handle_http_get(stream: &mut TcpStream, raw_path: &str, base_dir: &PathBuf) {
    let path_without_query = raw_path.split('?').next().unwrap_or(raw_path);
    let decoded = percent_encoding::percent_decode_str(path_without_query).decode_utf8_lossy().to_string();
    let file_path_str = decoded.trim_start_matches('/');
    
    let target = base_dir.join(file_path_str);
    let is_safe = target.canonicalize().and_then(|canon_target| {
        base_dir.canonicalize().map(|canon_base| canon_target.starts_with(&canon_base))
    }).unwrap_or(false);

    if !is_safe {
        let response = "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
        return;
    }
    
    match std::fs::read(&target) {
        Ok(content) => {
            let lower = target.to_string_lossy().to_lowercase();
            let mime = if lower.ends_with(".png") {
                "image/png"
            } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
                "image/jpeg"
            } else if lower.ends_with(".ico") {
                "image/x-icon"
            } else if lower.ends_with(".webp") {
                "image/webp"
            } else if lower.ends_with(".gif") {
                "image/gif"
            } else {
                "application/octet-stream"
            };

            let response = format!(
                "HTTP/1.1 200 OK\r\n\
                 Content-Type: {}\r\n\
                 Content-Length: {}\r\n\
                 Access-Control-Allow-Origin: *\r\n\
                 Connection: close\r\n\r\n",
                mime,
                content.len()
            );

            let _ = stream.write_all(response.as_bytes());
            let _ = stream.write_all(&content);
            let _ = stream.flush();
        }
        Err(_) => {
            let response = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    }
}


#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Game {
    pub id: String,
    pub name: String,
    pub exe_path: String,
    pub logo_path: Option<String>,
    pub banner_path: Option<String>,
    pub last_played: Option<String>,
    pub playtime_seconds: Option<u64>,
    #[serde(default)]
    pub date_added: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
pub struct LibraryData {
    pub games: Vec<Game>,
}

pub struct LibraryState {
    pub data_path: PathBuf,
}

impl LibraryState {
    pub fn new() -> Self {
        let path = match ProjectDirs::from("com", "MyLib", "MyLib") {
            Some(proj_dirs) => proj_dirs.data_local_dir().to_path_buf(),
            None => {
                eprintln!("Could not resolve a standard app-data directory; falling back to ./mylib-data");
                PathBuf::from("mylib-data")
            }
        };

        if let Err(e) = fs::create_dir_all(&path) {
            eprintln!("Failed to create app data dir '{}': {}", path.display(), e);
        }

        let file_path = path.join("games.json");
        Self { data_path: file_path }
    }

    pub fn load(&self) -> LibraryData {
        if self.data_path.exists() {
            match fs::read_to_string(&self.data_path) {
                Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
                    eprintln!("games.json was corrupt, starting with an empty library: {}", e);
                    LibraryData::default()
                }),
                Err(e) => {
                    eprintln!("Failed to read '{}': {}", self.data_path.display(), e);
                    LibraryData::default()
                }
            }
        } else {
            LibraryData::default()
        }
    }

    /// Returns true on success. Callers should avoid telling the UI a save
    /// succeeded (and thus discarding in-memory state) when this fails.
    pub fn save(&self, data: &LibraryData) -> bool {
        let content = match serde_json::to_string_pretty(data) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Failed to serialize library data: {}", e);
                return false;
            }
        };
        if let Some(parent) = self.data_path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                eprintln!("Failed to ensure data dir '{}' exists: {}", parent.display(), e);
            }
        }
        let tmp_path = self.data_path.with_extension("json.tmp");
        if let Err(e) = fs::write(&tmp_path, &content) {
            eprintln!("Failed to write tmp file '{}': {}", tmp_path.display(), e);
            return false;
        }
        match fs::rename(&tmp_path, &self.data_path) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("Failed to rename tmp file to '{}': {}", self.data_path.display(), e);
                false
            }
        }
    }

    pub fn game_assets_dir(&self, game_id: &str) -> PathBuf {
        get_base_games_dir().join(game_id)
    }
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum ResMessage {
    #[serde(rename = "init")]
    Init { games: Vec<Game>, port: u16 },
    #[serde(rename = "games_updated")]
    GamesUpdated { games: Vec<Game> },
    #[serde(rename = "exe_selected")]
    ExeSelected { path: Option<String> },
    #[serde(rename = "logo_selected")]
    LogoSelected { path: Option<String> },
    #[serde(rename = "banner_selected")]
    BannerSelected { path: Option<String> },
    #[serde(rename = "game_status")]
    GameStatus { id: String, status: String },
    #[serde(rename = "gamepad_input")]
    GamepadInput { action: String },
    #[serde(rename = "artwork_fetched")]
    ArtworkFetched { game_id: String, logo_path: Option<String>, banner_path: Option<String> },
    #[serde(rename = "artwork_error")]
    ArtworkError { error: String },
}

#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
enum ReqMessage {
    #[serde(rename = "ready")]
    Ready,
    #[serde(rename = "launch_game")]
    LaunchGame { id: String },
    #[serde(rename = "stop_game")]
    StopGame { id: String },
    #[serde(rename = "select_exe")]
    SelectExe,
    #[serde(rename = "select_logo")]
    SelectLogo,
    #[serde(rename = "select_banner")]
    SelectBanner,
    #[serde(rename = "save_game")]
    SaveGame {
        id: Option<String>,
        name: String,
        exe_path: String,
        logo_path: Option<String>,
        banner_path: Option<String>,
    },
    #[serde(rename = "delete_game")]
    DeleteGame {
        id: String,
    },
    #[serde(rename = "window_drag")]
    WindowDrag,
    #[serde(rename = "window_minimize")]
    WindowMinimize,
    #[serde(rename = "window_maximize")]
    WindowMaximize,
    #[serde(rename = "window_close")]
    WindowClose,
    #[serde(rename = "window_resize")]
    WindowResize { direction: String },
    #[serde(rename = "haptic")]
    Haptic { duration_ms: u32, strong: bool },
    #[serde(rename = "fetch_artwork")]
    FetchArtwork { game_id: Option<String>, name: String },
}

struct App {
    window: Option<Window>,
    webview: Option<wry::WebView>,
    state: Option<LibraryState>,
    proxy: winit::event_loop::EventLoopProxy<AppEvent>,
    running_games: HashMap<String, (std::time::Instant, std::time::Instant, Option<u32>)>,
    http_port: u16,
    sys: System,
    is_focused: bool,
    tray_icon: Option<tray_icon::TrayIcon>,
    cursor_pos: Option<winit::dpi::PhysicalPosition<f64>>,
    haptic_tx: std::sync::mpsc::Sender<(u32, bool)>,
    haptic_rx: Option<std::sync::mpsc::Receiver<(u32, bool)>>,
}

impl App {
    fn new(proxy: winit::event_loop::EventLoopProxy<AppEvent>, haptic_tx: std::sync::mpsc::Sender<(u32, bool)>, haptic_rx: std::sync::mpsc::Receiver<(u32, bool)>) -> Self {
        let http_port = start_http_server().expect("Failed to start local HTTP server");
        Self {
            window: None,
            webview: None,
            state: None,
            proxy,
            running_games: HashMap::new(),
            http_port,
            sys: System::new_all(),
            is_focused: true,
            tray_icon: None,
            cursor_pos: None,
            haptic_tx,
            haptic_rx: Some(haptic_rx),
        }
    }

    fn show_centered(&self) {
        if let Some(window) = &self.window {
            window.set_maximized(false);
            if let Some(monitor) = window.primary_monitor().or_else(|| window.current_monitor()) {
                let monitor_size = monitor.size();
                let window_size = window.outer_size();
                let x = (monitor_size.width as i32 - window_size.width as i32) / 2;
                let y = (monitor_size.height as i32 - window_size.height as i32) / 2;
                window.set_outer_position(winit::dpi::PhysicalPosition::new(x, y));
            }
            window.set_visible(true);
            window.focus_window();
        }
    }

    fn send_to_webview(&self, msg: &ResMessage) {
        if let Some(wv) = &self.webview {
            if let Ok(json) = serde_json::to_string(msg) {
                let js = format!("window.receiveMessage(String.raw`{}`);", json.replace("`", "\\`"));
                let _ = wv.evaluate_script(&js);
            }
        }
    }

    fn poll_running_games(&mut self) {
        if self.running_games.is_empty() {
            return;
        }

        self.sys.refresh_processes_specifics(sysinfo::ProcessesToUpdate::All, true, sysinfo::ProcessRefreshKind::nothing());
        
        let mut finished_ids = Vec::new();
        
        if let Some(state) = &mut self.state {
            let mut data = state.load();
            let mut data_changed = false;
            
            for (id, (start_time, last_update, tracked_pid)) in self.running_games.iter_mut() {
                if let Some(game) = data.games.iter_mut().find(|g| &g.id == id) {
                    let path = std::path::Path::new(&game.exe_path);
                    if let Some(file_name_os) = path.file_name() {
                        let exe_name = file_name_os.to_string_lossy().to_string();
                        let exe_name_lower = exe_name.to_lowercase();
                        
                        let mut is_running = false;
                        if let Some(pid) = tracked_pid {
                            if self.sys.process(sysinfo::Pid::from_u32(*pid)).is_some() {
                                is_running = true;
                            }
                        }
                        
                        if !is_running {
                            for proc in self.sys.processes().values() {
                                if proc.name().to_string_lossy().to_lowercase() == exe_name_lower {
                                    is_running = true;
                                    break;
                                }
                            }
                        }
                        
                        let elapsed_from_start = start_time.elapsed();
                        if !is_running {
                            if elapsed_from_start.as_secs() > 5 {
                                finished_ids.push(id.clone());
                            }
                        } else {
                            let elapsed = last_update.elapsed();
                            let add_secs = elapsed.as_secs();
                            if add_secs >= 1 {
                                game.playtime_seconds = Some(game.playtime_seconds.unwrap_or(0) + add_secs);
                                game.last_played = Some(chrono::Local::now().format("%Y-%m-%d").to_string());
                                *last_update += std::time::Duration::from_secs(add_secs);
                                data_changed = true;
                            }
                        }
                    } else {
                        finished_ids.push(id.clone());
                    }
                } else {
                    finished_ids.push(id.clone());
                }
            }
            
            if data_changed {
                state.save(&data);
                self.send_to_webview(&ResMessage::GamesUpdated { games: data.games });
            }
        }

        for id in finished_ids {
            self.running_games.remove(&id);
            self.send_to_webview(&ResMessage::GameStatus {
                id,
                status: "stopped".to_string(),
            });
        }
    }

    fn handle_ipc_message(&mut self, event: String) {
        if let Ok(req) = serde_json::from_str::<ReqMessage>(&event) {
            match req {
                ReqMessage::Ready => {
                    if let Some(state) = &self.state {
                        let data = state.load();
                        self.send_to_webview(&ResMessage::Init { games: data.games, port: self.http_port });
                        // Also notify of currently running status
                        for id in self.running_games.keys() {
                            self.send_to_webview(&ResMessage::GameStatus {
                                id: id.clone(),
                                status: "running".to_string(),
                            });
                        }
                    }
                }
                ReqMessage::LaunchGame { id } => {
                    if let Some(state) = &self.state {
                        let data = state.load();
                        if let Some(game) = data.games.iter().find(|g| g.id == id) {
                            if self.running_games.contains_key(&id) {
                                return;
                            }
                            let path = std::path::Path::new(&game.exe_path);
                            let current_dir = path.parent().unwrap_or(std::path::Path::new(""));
                            
                            let mut cmd = std::process::Command::new(&game.exe_path);
                            if current_dir.exists() && current_dir.is_dir() {
                                cmd.current_dir(current_dir);
                            }
                            
                            use std::os::windows::process::CommandExt;
                            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
                            
                            match cmd.spawn() {
                                Ok(child) => {
                                    println!("Successfully launched {}", game.name);
                                    let now = std::time::Instant::now();
                                    self.running_games.insert(id.clone(), (now, now, Some(child.id())));
                                    self.send_to_webview(&ResMessage::GameStatus {
                                        id: id.clone(),
                                        status: "running".to_string(),
                                    });
                                }
                                Err(e) => {
                                    eprintln!("Failed to launch natively {}: {}", game.exe_path, e);
                                    let script = format!("Start-Process '{}' -Verb RunAs", game.exe_path.replace("'", "''"));
                                    let mut ps = std::process::Command::new("powershell");
                                    ps.creation_flags(0x08000000); // Hide powershell terminal window
                                    ps.args(&["-NoProfile", "-Command", &script]);
                                    if current_dir.exists() && current_dir.is_dir() {
                                        ps.current_dir(current_dir);
                                    }
                                    if let Ok(_) = ps.spawn() {
                                        let now = std::time::Instant::now();
                                        self.running_games.insert(id.clone(), (now, now, None));
                                        self.send_to_webview(&ResMessage::GameStatus {
                                            id: id.clone(),
                                            status: "running".to_string(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
                ReqMessage::StopGame { id } => {
                    if let Some((_start_time, last_update, pid_opt)) = self.running_games.remove(&id) {
                        if let Some(state) = &mut self.state {
                            let mut data = state.load();
                            if let Some(game) = data.games.iter_mut().find(|g| g.id == id) {
                                self.sys.refresh_processes_specifics(sysinfo::ProcessesToUpdate::All, true, sysinfo::ProcessRefreshKind::nothing());
                                
                                if let Some(pid) = pid_opt {
                                    let root_pid = sysinfo::Pid::from_u32(pid);
                                    let mut to_kill = vec![root_pid];
                                    
                                    // Find descendants
                                    for (p_id, proc) in self.sys.processes() {
                                        if let Some(parent_id) = proc.parent() {
                                            if to_kill.contains(&parent_id) {
                                                to_kill.push(*p_id);
                                            }
                                        }
                                    }
                                    
                                    for p_id in to_kill {
                                        if let Some(proc) = self.sys.process(p_id) {
                                            proc.kill();
                                        }
                                    }
                                } else {
                                    // Fallback to name if PID was not tracked (e.g. powershell launch)
                                    let path = std::path::Path::new(&game.exe_path);
                                    if let Some(file_name_os) = path.file_name() {
                                        let exe_name = file_name_os.to_string_lossy().to_string();
                                        let exe_name_lower = exe_name.to_lowercase();
                                        
                                        for proc in self.sys.processes().values() {
                                            if proc.name().to_string_lossy().to_lowercase() == exe_name_lower {
                                                proc.kill();
                                            }
                                        }
                                    }
                                }

                                let elapsed = last_update.elapsed();
                                let add_secs = elapsed.as_secs();
                                if add_secs > 0 {
                                    game.playtime_seconds = Some(game.playtime_seconds.unwrap_or(0) + add_secs);
                                    game.last_played = Some(chrono::Local::now().format("%Y-%m-%d").to_string());
                                    state.save(&data);
                                    self.send_to_webview(&ResMessage::GamesUpdated { games: data.games });
                                }
                            }
                        }
                        
                        self.send_to_webview(&ResMessage::GameStatus {
                            id: id.clone(),
                            status: "stopped".to_string(),
                        });
                    }
                }
                ReqMessage::SelectExe => {
                    if let Some(path) = rfd::FileDialog::new().add_filter("exe", &["exe", "bat"]).pick_file() {
                        self.send_to_webview(&ResMessage::ExeSelected { path: Some(path.to_string_lossy().to_string()) });
                    } else {
                        self.send_to_webview(&ResMessage::ExeSelected { path: None });
                    }
                }
                ReqMessage::SelectLogo => {
                    if let Some(path) = rfd::FileDialog::new().add_filter("image", &["png", "jpg", "jpeg", "ico"]).pick_file() {
                        self.send_to_webview(&ResMessage::LogoSelected { path: Some(path.to_string_lossy().to_string()) });
                    } else {
                        self.send_to_webview(&ResMessage::LogoSelected { path: None });
                    }
                }
                ReqMessage::SelectBanner => {
                    if let Some(path) = rfd::FileDialog::new().add_filter("image", &["png", "jpg", "jpeg"]).pick_file() {
                        self.send_to_webview(&ResMessage::BannerSelected { path: Some(path.to_string_lossy().to_string()) });
                    } else {
                        self.send_to_webview(&ResMessage::BannerSelected { path: None });
                    }
                }
                ReqMessage::SaveGame { id, name, exe_path, logo_path, banner_path } => {
                    if let Some(state) = &self.state {
                        let mut data = state.load();
                        let is_new = id.is_none();
                        let new_id = id.unwrap_or_else(|| Uuid::new_v4().to_string());
                        let game_dir = state.game_assets_dir(&new_id);
                        let _ = std::fs::create_dir_all(&game_dir);

                        let existing_idx = data.games.iter().position(|g| g.id == new_id);
                        let existing = existing_idx.map(|idx| data.games[idx].clone());

                        let mut stored_logo = existing.as_ref().and_then(|g| g.logo_path.clone());
                        let mut stored_banner = existing.as_ref().and_then(|g| g.banner_path.clone());

                        if let Some(ref path) = logo_path {
                            if path != stored_logo.as_deref().unwrap_or("") {
                                if let Ok(entries) = std::fs::read_dir(&game_dir) {
                                    for entry in entries.flatten() {
                                        if entry.path().file_stem().and_then(|s| s.to_str()) == Some("logo") {
                                            let _ = std::fs::remove_file(entry.path());
                                        }
                                    }
                                }
                                if !path.is_empty() && std::path::Path::new(path).exists() {
                                    let ext = std::path::Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("png").to_lowercase();
                                    let final_logo_path = game_dir.join(format!("logo.{}", ext));
                                    if std::fs::copy(path, &final_logo_path).is_ok() {
                                        stored_logo = Some(final_logo_path.to_string_lossy().to_string());
                                    } else {
                                        stored_logo = None;
                                    }
                                } else {
                                    stored_logo = None;
                                }
                            }
                        }

                        if let Some(ref path) = banner_path {
                            if path != stored_banner.as_deref().unwrap_or("") {
                                if let Ok(entries) = std::fs::read_dir(&game_dir) {
                                    for entry in entries.flatten() {
                                        if entry.path().file_stem().and_then(|s| s.to_str()) == Some("banner") {
                                            let _ = std::fs::remove_file(entry.path());
                                        }
                                    }
                                }
                                if !path.is_empty() && std::path::Path::new(path).exists() {
                                    let ext = std::path::Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("png").to_lowercase();
                                    let final_banner_path = game_dir.join(format!("banner.{}", ext));
                                    if std::fs::copy(path, &final_banner_path).is_ok() {
                                        stored_banner = Some(final_banner_path.to_string_lossy().to_string());
                                    } else {
                                        stored_banner = None;
                                    }
                                } else {
                                    stored_banner = None;
                                }
                            }
                        }

                        let old_exe = existing.as_ref().map(|g| g.exe_path.as_str()).unwrap_or("");
                        if stored_logo.is_none() && std::path::Path::new(&exe_path).exists() && (is_new || exe_path != old_exe) {
                            let final_logo_path = game_dir.join("logo.png");
                            let script = format!(
                                "Add-Type -AssemblyName System.Drawing; try {{ $icon = [System.Drawing.Icon]::ExtractAssociatedIcon('{}'); if ($icon) {{ $bitmap = $icon.ToBitmap(); $bitmap.Save('{}', [System.Drawing.Imaging.ImageFormat]::Png); exit 0 }} }} catch {{ Write-Error $_ }} exit 1",
                                exe_path.replace("'", "''"),
                                final_logo_path.to_string_lossy().replace("'", "''")
                            );
                            let mut ps = Command::new("powershell");
                            ps.creation_flags(0x08000000);
                            ps.args(&["-NoProfile", "-Command", &script]);
                            let _ = ps.output();
                            if final_logo_path.exists() {
                                stored_logo = Some(final_logo_path.to_string_lossy().to_string());
                            }
                        }

                        let now_date = chrono::Local::now().format("%Y-%m-%d").to_string();

                        let game = Game {
                            id: new_id.clone(),
                            name,
                            exe_path,
                            logo_path: stored_logo,
                            banner_path: stored_banner,
                            last_played: existing.as_ref().and_then(|g| g.last_played.clone()),
                            playtime_seconds: existing.as_ref().and_then(|g| g.playtime_seconds),
                            date_added: existing.as_ref().and_then(|g| g.date_added.clone()).or(Some(now_date)),
                        };

                        if let Some(pos) = existing_idx {
                            data.games[pos] = game;
                        } else {
                            data.games.push(game);
                        }

                        if state.save(&data) {
                            self.send_to_webview(&ResMessage::GamesUpdated { games: data.games });
                        } else {
                            self.send_to_webview(&ResMessage::GamesUpdated { games: state.load().games });
                        }
                    }
                }
                ReqMessage::DeleteGame { id } => {
                    if let Some(state) = &self.state {
                        let mut data = state.load();
                        if let Some(pos) = data.games.iter().position(|g| g.id == id) {
                            data.games.remove(pos);
                            if state.save(&data) {
                                let game_dir = state.game_assets_dir(&id);
                                if let Err(e) = std::fs::remove_dir_all(&game_dir) {
                                    eprintln!("Failed to remove asset dir '{}': {}", game_dir.display(), e);
                                }
                                self.send_to_webview(&ResMessage::GamesUpdated { games: data.games });
                            } else {
                                eprintln!("delete_game: library write failed, leaving entry in place on disk");
                                self.send_to_webview(&ResMessage::GamesUpdated { games: state.load().games });
                            }
                        }
                    }
                }
                ReqMessage::WindowDrag => {
                    if let Some(window) = &self.window {
                        if window.is_maximized() {
                            window.set_maximized(false);
                            if let Some(cursor_pos) = self.cursor_pos {
                                // Calculate the proportional X position of the cursor
                                // If the window restores to 800 width, we want the cursor to roughly be in the same relative X
                                // However, simple approach: just put the window's top center at the cursor.
                                let size = window.outer_size();
                                let mut pos = cursor_pos.clone();
                                // Assuming titlebar height is ~32px, place top-left so cursor is in the titlebar.
                                pos.x -= (size.width as f64) / 2.0;
                                pos.y -= 10.0;
                                window.set_outer_position(winit::dpi::PhysicalPosition::new(pos.x as i32, pos.y as i32));
                            }
                        }
                        let _ = window.drag_window();
                    }
                }
                ReqMessage::WindowMinimize => {
                    if let Some(window) = &self.window {
                        window.set_minimized(true);
                    }
                }
                ReqMessage::WindowMaximize => {
                    if let Some(window) = &self.window {
                        let is_maximized = window.is_maximized();
                        window.set_maximized(!is_maximized);
                    }
                }
                ReqMessage::WindowClose => {
                    let _ = self.proxy.send_event(AppEvent::CloseWindow);
                }
                ReqMessage::WindowResize { direction } => {
                    if let Some(window) = &self.window {
                        let dir = match direction.as_str() {
                            "e" => winit::window::ResizeDirection::East,
                            "n" => winit::window::ResizeDirection::North,
                            "ne" => winit::window::ResizeDirection::NorthEast,
                            "nw" => winit::window::ResizeDirection::NorthWest,
                            "s" => winit::window::ResizeDirection::South,
                            "se" => winit::window::ResizeDirection::SouthEast,
                            "sw" => winit::window::ResizeDirection::SouthWest,
                            "w" => winit::window::ResizeDirection::West,
                            _ => return,
                        };
                        let _ = window.drag_resize_window(dir);
                    }
                }
                ReqMessage::Haptic { duration_ms, strong } => {
                    let _ = self.haptic_tx.send((duration_ms, strong));
                }
                ReqMessage::FetchArtwork { game_id, name } => {
                    let mut game_dir = None;
                    if let Some(state) = &self.state {
                        let g_id = game_id.clone().unwrap_or_else(|| Uuid::new_v4().to_string());
                        game_dir = Some((g_id.clone(), state.game_assets_dir(&g_id)));
                    }
                    let proxy = self.proxy.clone();
                    std::thread::spawn(move || {
                        let (g_id, dir) = match game_dir {
                            Some(d) => d,
                            None => return,
                        };
                        let key = "74582d73206b61c9a8073bdcccdc9136";
                        
                        #[derive(Deserialize)] struct SearchRes { data: Vec<SgdGame> }
                        #[derive(Deserialize)] struct SgdGame { id: u32 }
                        #[derive(Deserialize)] struct ImageRes { data: Vec<SgdImage> }
                        #[derive(Deserialize)] struct SgdImage { url: String }
                        let client = match reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(15)).build() {
                            Ok(c) => c,
                            Err(e) => {
                                println!("Failed to build reqwest client: {}", e);
                                let _ = proxy.send_event(AppEvent::DispatchToWebView(serde_json::to_string(&ResMessage::ArtworkError { error: format!("Internal client error: {}", e) }).unwrap()));
                                return;
                            }
                        };
                        
                        let url = format!("https://www.steamgriddb.com/api/v2/search/autocomplete/{}", percent_encoding::percent_encode(name.as_bytes(), percent_encoding::NON_ALPHANUMERIC));
                        println!("Fetching artwork for: {} at {}", name, url);
                        
                        let resp = match client.get(&url).header("Authorization", format!("Bearer {}", key)).send() {
                            Ok(r) => r,
                            Err(e) => {
                                println!("Reqwest error: {}", e);
                                let _ = proxy.send_event(AppEvent::DispatchToWebView(serde_json::to_string(&ResMessage::ArtworkError { error: format!("Network error: {}", e) }).unwrap()));
                                return;
                            }
                        };
                        println!("Search API status: {}", resp.status());
                        let s_res: SearchRes = match resp.json() {
                            Ok(j) => j,
                            Err(e) => {
                                println!("Search JSON parse error: {}", e);
                                let _ = proxy.send_event(AppEvent::DispatchToWebView(serde_json::to_string(&ResMessage::ArtworkError { error: "Failed to parse search response".into() }).unwrap()));
                                return;
                            }
                        };
                        let sgd_id = match s_res.data.first() {
                            Some(g) => {
                                println!("Found game ID: {}", g.id);
                                g.id
                            },
                            None => {
                                println!("Game not found in SearchRes");
                                let _ = proxy.send_event(AppEvent::DispatchToWebView(serde_json::to_string(&ResMessage::ArtworkError { error: "Game not found on SteamGridDB".into() }).unwrap()));
                                return;
                            }
                        };

                        let _ = std::fs::create_dir_all(&dir);

                        let mut final_logo = None;
                        println!("Fetching logo...");
                        if let Ok(resp) = client.get(&format!("https://www.steamgriddb.com/api/v2/logos/game/{}?styles=official,custom", sgd_id)).header("Authorization", format!("Bearer {}", key)).send() {
                            if let Ok(res) = resp.json::<ImageRes>() {
                                if let Some(img) = res.data.first() {
                                    println!("Found logo URL: {}", img.url);
                                    if let Ok(mut r) = client.get(&img.url).send() {
                                        let mut buf = Vec::new();
                                        if std::io::Read::read_to_end(&mut r, &mut buf).is_ok() {
                                            let ext = img.url.split('.').last().unwrap_or("png");
                                            let path = dir.join(format!("logo.{}", ext));
                                            if std::fs::write(&path, &buf).is_ok() {
                                                println!("Logo saved to {:?}", path);
                                                final_logo = Some(path.to_string_lossy().to_string());
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        let mut final_banner = None;
                        println!("Fetching banner...");
                        if let Ok(resp) = client.get(&format!("https://www.steamgriddb.com/api/v2/heroes/game/{}?styles=alternate,material", sgd_id)).header("Authorization", format!("Bearer {}", key)).send() {
                            if let Ok(res) = resp.json::<ImageRes>() {
                                if let Some(img) = res.data.first() {
                                    println!("Found banner URL: {}", img.url);
                                    if let Ok(mut r) = client.get(&img.url).send() {
                                        let mut buf = Vec::new();
                                        if std::io::Read::read_to_end(&mut r, &mut buf).is_ok() {
                                            let ext = img.url.split('.').last().unwrap_or("png");
                                            let path = dir.join(format!("banner.{}", ext));
                                            if std::fs::write(&path, &buf).is_ok() {
                                                println!("Banner saved to {:?}", path);
                                                final_banner = Some(path.to_string_lossy().to_string());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        
                        println!("Sending ArtworkFetched to UI");
                        
                        let msg = ResMessage::ArtworkFetched { game_id: g_id, logo_path: final_logo, banner_path: final_banner };
                        let _ = proxy.send_event(AppEvent::DispatchToWebView(serde_json::to_string(&msg).unwrap()));
                    });
                }
            }
        }
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            self.state = Some(LibraryState::new());
            
            let icon_bytes = include_bytes!("icon.png");
            let img = image::load_from_memory(icon_bytes).expect("Failed to load icon").into_rgba8();
            let (width, height) = img.dimensions();
            let rgba = img.into_raw();
            let window_icon = winit::window::Icon::from_rgba(rgba.clone(), width, height).unwrap();
            let tray_icon_img = tray_icon::Icon::from_rgba(rgba, width, height).unwrap();

            let window_attrs = Window::default_attributes()
                .with_title("MyLib")
                .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0))
                .with_min_inner_size(winit::dpi::LogicalSize::new(900.0, 560.0))
                .with_decorations(false)
                .with_undecorated_shadow(true)
                .with_window_icon(Some(window_icon))
                .with_corner_preference(CornerPreference::Round)
                .with_visible(false);
                
            let window = event_loop
                .create_window(window_attrs)
                .expect("failed to create OS window");
            
            let show_i = tray_icon::menu::MenuItem::with_id("show", "Show MyLib", true, None);
            let quit_i = tray_icon::menu::MenuItem::with_id("quit", "Quit", true, None);
            let menu = tray_icon::menu::Menu::new();
            let _ = menu.append(&show_i);
            let _ = menu.append(&quit_i);
            
            let tray_icon = tray_icon::TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_tooltip("MyLib")
                .with_icon(tray_icon_img)
                .build()
                .ok();
            self.tray_icon = tray_icon;

            let proxy = self.proxy.clone();
            
            // Start the process check thread
            let poll_proxy = self.proxy.clone();
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    if poll_proxy.send_event(AppEvent::PollGames).is_err() {
                        break;
                    }
                }
            });
            
            let gamepad_proxy = self.proxy.clone();
            let h_rx = self.haptic_rx.take().unwrap();
            std::thread::spawn(move || {
                start_gilrs_polling(gamepad_proxy, h_rx);
            });
            
            let builder = WebViewBuilder::new();
            
            let webview = builder
                .with_html(include_str!("index.html"))
                .with_ipc_handler(move |msg: wry::http::Request<String>| {
                    let _ = proxy.send_event(AppEvent::Ipc(msg.into_body()));
                })
                .with_custom_protocol("asset".into(), move |_id, request| {
                    let path = request.uri().path().to_string();
                    let decoded = percent_encoding::percent_decode_str(&path).decode_utf8_lossy().to_string();
                    let file_path_str = decoded.trim_start_matches('/');
                    
                    let base_dir = get_base_games_dir();
                    let target = base_dir.join(file_path_str);
                    let is_safe = target.canonicalize().and_then(|canon_target| {
                        base_dir.canonicalize().map(|canon_base| canon_target.starts_with(&canon_base))
                    }).unwrap_or(false);

                    if !is_safe {
                        return wry::http::Response::builder()
                            .status(403)
                            .header("Access-Control-Allow-Origin", "*")
                            .body(Vec::new().into())
                            .expect("building a 403 response must not fail");
                    }

                    match std::fs::read(&target) {
                        Ok(content) => {
                            let mut response = wry::http::Response::builder()
                                .status(200)
                                .header("Access-Control-Allow-Origin", "*");

                            let lower = target.to_string_lossy().to_lowercase();
                            if lower.ends_with(".png") {
                                response = response.header("Content-Type", "image/png");
                            } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
                                response = response.header("Content-Type", "image/jpeg");
                            } else if lower.ends_with(".ico") {
                                response = response.header("Content-Type", "image/x-icon");
                            } else if lower.ends_with(".webp") {
                                response = response.header("Content-Type", "image/webp");
                            } else if lower.ends_with(".gif") {
                                response = response.header("Content-Type", "image/gif");
                            }

                            response.body(content.into()).unwrap_or_else(|e| {
                                eprintln!("asset protocol: failed to build response for {}: {}", file_path_str, e);
                                wry::http::Response::builder()
                                    .status(500)
                                    .body(Vec::new().into())
                                     .expect("building a fallback empty response must not fail")
                            })
                        }
                        Err(e) => {
                            eprintln!("asset protocol: could not read '{}': {}", file_path_str, e);
                            wry::http::Response::builder()
                                .status(404)
                                .header("Access-Control-Allow-Origin", "*")
                                .body(Vec::new().into())
                                .expect("building a 404 response must not fail")
                        }
                    }
                })
                .build(&window)
                .expect("failed to initialize the embedded webview");
                
            self.window = Some(window);
            self.webview = Some(webview);
            self.show_centered();
        }
    }

    fn window_event(&mut self, _event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                if let Some(window) = &self.window {
                    window.set_visible(false);
                }
            }
            WindowEvent::Focused(focused) => {
                self.is_focused = focused;
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_pos = Some(position);
            }
            _ => {}
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::Ipc(msg_str) => {
                self.handle_ipc_message(msg_str);
            }
            AppEvent::DispatchToWebView(json) => {
                if let Some(wv) = &self.webview {
                    let js = format!("window.receiveMessage(String.raw`{}`);", json.replace("`", "\\`"));
                    let _ = wv.evaluate_script(&js);
                }
            }
            AppEvent::PollGames => {
                self.poll_running_games();
                
                if let Ok(event) = tray_icon::TrayIconEvent::receiver().try_recv() {
                    if let tray_icon::TrayIconEvent::Click { button: tray_icon::MouseButton::Left, button_state: tray_icon::MouseButtonState::Up, .. } = event {
                        if let Some(window) = &self.window {
                            if window.is_visible().unwrap_or(false) && self.is_focused {
                                window.set_visible(false);
                            } else {
                                self.show_centered();
                            }
                        }
                    }
                }
                if let Ok(event) = tray_icon::menu::MenuEvent::receiver().try_recv() {
                    if event.id.0 == "show" {
                        self.show_centered();
                    } else if event.id.0 == "quit" {
                        event_loop.exit();
                    }
                }
            }
            AppEvent::GamepadInput(action) => {
                if self.is_focused {
                    self.send_to_webview(&ResMessage::GamepadInput { action });
                }
            }
            AppEvent::CloseWindow => {
                if let Some(window) = &self.window {
                    window.set_visible(false);
                }
            }
            AppEvent::ToggleWindow => {
                if let Some(window) = &self.window {
                    if window.is_visible().unwrap_or(false) && self.is_focused {
                        window.set_visible(false);
                    } else {
                        self.show_centered();
                    }
                }
            }
        }
    }
}

pub fn main() {
    if !is_elevated::is_elevated() {
        let exe = std::env::current_exe().unwrap();
        let _ = runas::Command::new(exe).status();
        std::process::exit(0);
    }

    let mut sys = sysinfo::System::new_all();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let my_pid = std::process::id();
    for (pid, proc) in sys.processes() {
        if pid.as_u32() != my_pid {
            let name = proc.name().to_string_lossy().to_lowercase();
            if name == "mylib.exe" {
                proc.kill();
            }
        }
    }

    let event_loop = EventLoop::<AppEvent>::with_user_event().build().unwrap();
    event_loop.set_control_flow(ControlFlow::Wait);

    let proxy = event_loop.create_proxy();
    let (haptic_tx, haptic_rx) = std::sync::mpsc::channel();
    let mut app = App::new(proxy, haptic_tx, haptic_rx);
    
    event_loop.run_app(&mut app).unwrap();
}

fn start_gilrs_polling(proxy: winit::event_loop::EventLoopProxy<AppEvent>, haptic_rx: std::sync::mpsc::Receiver<(u32, bool)>) {
    use gilrs::{Gilrs, Event, EventType, Button};
    use gilrs::ff::{EffectBuilder, Replay, BaseEffect, BaseEffectType, Ticks};
    let mut gilrs = match Gilrs::new() {
        Ok(g) => g,
        Err(_) => return,
    };
    
    // Simple state tracking for axes to send discrete D-pad like events
    let mut last_axis_up = false;
    let mut last_axis_down = false;
    let mut last_axis_left = false;
    let mut last_axis_right = false;

    let mut last_active_gamepad = None;
    let mut _active_effect: Option<gilrs::ff::Effect> = None;

    loop {
        while let Some(Event { id, event, .. }) = gilrs.next_event() {
            last_active_gamepad = Some(id);
            match event {
                EventType::ButtonPressed(Button::DPadUp, _) => { let _ = proxy.send_event(AppEvent::GamepadInput("up".to_string())); }
                EventType::ButtonPressed(Button::DPadDown, _) => { let _ = proxy.send_event(AppEvent::GamepadInput("down".to_string())); }
                EventType::ButtonPressed(Button::DPadLeft, _) => { let _ = proxy.send_event(AppEvent::GamepadInput("left".to_string())); }
                EventType::ButtonPressed(Button::DPadRight, _) => { let _ = proxy.send_event(AppEvent::GamepadInput("right".to_string())); }
                EventType::ButtonPressed(Button::South, _) => { let _ = proxy.send_event(AppEvent::GamepadInput("a".to_string())); }
                EventType::ButtonPressed(Button::East, _) => { let _ = proxy.send_event(AppEvent::GamepadInput("b".to_string())); }
                EventType::ButtonPressed(Button::West, _) => { let _ = proxy.send_event(AppEvent::GamepadInput("x".to_string())); }
                EventType::ButtonPressed(Button::Mode, _) => { let _ = proxy.send_event(AppEvent::ToggleWindow); }
                EventType::AxisChanged(gilrs::Axis::LeftStickY, val, _) => {
                    let deadzone = 0.5;
                    let up = val > deadzone;
                    let down = val < -deadzone;
                    
                    if up && !last_axis_up { let _ = proxy.send_event(AppEvent::GamepadInput("up".to_string())); }
                    if down && !last_axis_down { let _ = proxy.send_event(AppEvent::GamepadInput("down".to_string())); }
                    
                    last_axis_up = up;
                    last_axis_down = down;
                }
                EventType::AxisChanged(gilrs::Axis::LeftStickX, val, _) => {
                    let deadzone = 0.5;
                    let right = val > deadzone;
                    let left = val < -deadzone;
                    
                    if right && !last_axis_right { let _ = proxy.send_event(AppEvent::GamepadInput("right".to_string())); }
                    if left && !last_axis_left { let _ = proxy.send_event(AppEvent::GamepadInput("left".to_string())); }
                    
                    last_axis_right = right;
                    last_axis_left = left;
                }
                _ => {}
            }
        }
        
        while let Ok((duration_ms, strong)) = haptic_rx.try_recv() {
            if let Some(gp_id) = last_active_gamepad {
                let mag = if strong { 60_000 } else { 20_000 };
                let effect = EffectBuilder::new()
                    .add_effect(BaseEffect {
                        kind: BaseEffectType::Strong { magnitude: mag },
                        scheduling: Replay { play_for: Ticks::from_ms(duration_ms), ..Default::default() },
                        envelope: Default::default()
                    })
                    .gamepads(&[gp_id])
                    .finish(&mut gilrs);
                if let Ok(eff) = effect {
                    let _ = eff.play();
                    _active_effect = Some(eff);
                }
            }
        }
        
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
}
