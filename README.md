<div align="center">
  <img src="https://i.ibb.co/G4pZFTNK/Screenshot-2026-07-01-102209.png" alt="MyLib Interface">
  <br>
  <h1>MyLib</h1>
  <p><b>A sleek, minimalist, and deeply integrated local game launcher.</b></p>
</div>

---

## 📌 Overview

**MyLib** is a lightweight, aesthetically driven desktop application that acts as a unified library for your local games. Built with Rust and Tauri/Wry, it provides a seamless and visually stunning interface to manage, launch, and track your game collection.

Designed to be unobtrusive, MyLib sits gracefully in your system tray and offers full gamepad support alongside standard keyboard and mouse navigation. 

## ✨ Features

- **Beautiful UI:** A dark, premium interface with dynamic background blurring, custom artwork, and smooth micro-animations.
- **Automated Artwork Fetching:** Automatically retrieves high-quality logos and banners from SteamGridDB upon adding a game.
- **Full Gamepad Support:** Navigate, manage, and launch your games directly from your controller with integrated haptic feedback.
- **Smart Execution:** Launches games efficiently via Rust's process management, handling administrative privilege escalation through PowerShell automatically when necessary.
- **System Tray Integration:** Minimizes cleanly to the system tray, waking instantly on command or hotkey.
- **Playtime Tracking:** Automatically tracks your playtime for each launched game.

## 🚀 Getting Started

### Prerequisites
- [Rust](https://www.rust-lang.org/tools/install) (latest stable)

### Building from Source

1. **Clone the repository:**
   ```sh
   git clone https://github.com/nikolaus018/MyLib.git
   cd MyLib
   ```

2. **Run the application:**
   ```sh
   cargo run --release
   ```
   *Note: For the best experience, ensure you run the release build. Debug builds may have slower UI performance.*

## 🎮 Usage

### Adding Games
Click the **+ Add Game** button (or press `E` on an empty slot). Browse for your game's executable. MyLib will attempt to automatically fetch the appropriate banner and logo metadata from SteamGridDB. 

### Navigation
- **Keyboard:** Use the `Arrow Keys` to browse, `Enter` to play, and `E` to edit game properties.
- **Gamepad:** Use the `D-Pad` or `Left Stick` to navigate, `A` to launch, and `Y` to edit.

### The "Elegant Launch"
When launching a game, MyLib displays a seamless "Launching..." overlay, masking your desktop before gracefully minimizing to the system tray, ensuring your immersion is never broken.

## 🛠️ Architecture

- **Backend:** Rust (`winit`, `wry`, `tray-icon`, `sysinfo`)
- **Frontend:** Vanilla HTML, CSS, and JS. No bloated frameworks.
- **Communication:** Custom IPC protocol and an `asset://` URI scheme for blazing-fast local image loading.

## 🤝 Contributing

Contributions, issues, and feature requests are welcome! Feel free to check the [issues page](https://github.com/nikolaus018/MyLib/issues).

## 📄 License

This project is licensed under the MIT License.
