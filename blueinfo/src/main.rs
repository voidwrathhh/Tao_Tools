// ======================== 命令行入口 ========================
//
// 使用 clap 实现多子命令管理。
// `cargo run -- bluetooth-info` 启动 GUI 图形界面。
// 后续可扩展其他子命令与现有工具链集成。

mod comm;
mod config;
mod data_struct;
mod gui;

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
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::BluetoothInfo => run_gui(),
    }
}

fn run_gui() {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([960.0, 600.0])
            .with_title("BlueInfo - 蓝牙调试工具"),
        ..Default::default()
    };

    let result = eframe::run_native(
        "BlueInfo - 蓝牙调试工具",
        options,
        Box::new(|_cc| Ok(Box::new(gui::BlueInfoApp::new()))),
    );

    if let Err(e) = result {
        eprintln!("错误：GUI 启动失败: {}", e);
        std::process::exit(1);
    }
}
