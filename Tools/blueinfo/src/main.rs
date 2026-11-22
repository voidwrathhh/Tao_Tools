// ======================== 命令行入口 ========================
//
// 使用 clap 实现多子命令管理。
// `cargo run -- bluetooth-info` 启动 GUI 图形界面。
// 后续可扩展其他子命令与现有工具链集成。

mod comm;
mod config;
mod data_struct;
mod gui;
mod protocol;

use clap::{Parser, Subcommand};
use eframe::egui;

#[derive(Parser)]
#[command(name = "blueinfo", about = "Tao Tools - 蓝牙调试工具", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 启动桌面蓝牙调试图形界面
    BluetoothInfo,
    /// 只打开设置窗口（不连接设备，供 toolhub 设置直达）
    Settings,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::BluetoothInfo => run_gui(false),
        Commands::Settings => run_gui(true),
    }
}

fn run_gui(settings_only: bool) {
    let (size, min_size, title) = if settings_only {
        ([680.0, 720.0], [520.0, 480.0], "BlueInfo 设置")
    } else {
        ([1280.0, 800.0], [960.0, 600.0], "BlueInfo - 蓝牙调试工具")
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(size)
            .with_min_inner_size(min_size)
            .with_title(title),
        ..Default::default()
    };

    let result = eframe::run_native(
        title,
        options,
        Box::new(move |_cc| Ok(Box::new(gui::BlueInfoApp::new(settings_only)))),
    );

    if let Err(e) = result {
        eprintln!("错误：GUI 启动失败: {}", e);
        std::process::exit(1);
    }
}
