// ======================== 命令行入口 ========================
//
// toolhub 是 tao_tools 工具中心的图形入口：
//   toolhub              打开工具中心界面
//   toolhub list         在终端列出已注册工具（含可执行文件解析结果）
//   toolhub registry     打印当前生效的注册表路径
//   toolhub open-registry 用系统编辑器打开注册表
//
// 新工具必须注册到 toolhub/registry.json（见 README.md）。

mod config;
mod gui;
mod launcher;
mod registry;

use std::process;

const APP_NAME: &str = "toolhub";
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        None | Some("gui") => gui::run(),
        Some("list") => list_tools(),
        Some("registry") => {
            let registry = registry::Registry::load();
            match registry.source.file_path() {
                Some(path) => println!("{}", path.display()),
                None => println!("{}", registry.source.label()),
            }
        }
        Some("open-registry") => {
            let registry = registry::Registry::load();
            match registry.source.file_path() {
                Some(path) => {
                    if let Err(e) = launcher::open_path(path) {
                        eprintln!("错误：{}", e);
                        process::exit(1);
                    }
                }
                None => {
                    eprintln!("当前使用内置注册表，没有可编辑的文件。");
                    process::exit(1);
                }
            }
        }
        Some("--help") | Some("-h") => print_usage(),
        Some("--version") | Some("-v") => println!("{} v{}", APP_NAME, VERSION),
        Some(other) => {
            eprintln!("错误：未知命令 '{}'", other);
            print_usage();
            process::exit(1);
        }
    }
}

fn list_tools() {
    let registry = registry::Registry::load();
    let config = config::HubConfig::load();
    let tools = config.apply_order(&registry.tools);

    if tools.is_empty() {
        println!("注册表为空：{}", registry.source.label());
        return;
    }

    println!("注册表：{}", registry.source.label());
    println!("{:<10} {:<8} {:<6} {}", "工具", "版本", "类型", "可执行文件");
    for tool in &tools {
        println!(
            "{:<10} v{:<7} {:<6} {}",
            tool.id,
            tool.version,
            tool.kind,
            launcher::describe(tool, &config)
        );
    }
}

fn print_usage() {
    println!("{} v{} - Tao Tools 工具中心", APP_NAME, VERSION);
    println!();
    println!("用法：");
    println!("  {}                              打开工具中心界面", APP_NAME);
    println!("  {} list                         列出已注册工具", APP_NAME);
    println!("  {} registry                     显示当前生效的注册表", APP_NAME);
    println!("  {} open-registry                打开注册表文件", APP_NAME);
    println!("  {} --version, -v                显示版本", APP_NAME);
    println!("  {} --help, -h                   显示帮助", APP_NAME);
    println!();
    println!("新增工具后必须注册到 toolhub/registry.json，详见 toolhub/README.md。");
}
