// ======================== 命令行入口 ========================
//
// toolhub 是 tao_tools 工具中心的图形入口：
//   toolhub              打开工具中心界面
//   toolhub list         在终端列出已注册工具（含可执行文件解析结果）
//   toolhub check        体检：版本不一致、可执行文件缺失、未注册的 workspace 包
//   toolhub registry     打印当前生效的注册表路径
//   toolhub open-registry 用系统编辑器打开注册表
//
// 新工具必须注册到 toolhub/registry.json（见 README.md）。

mod config;
mod gui;
mod health;
mod launcher;
mod metadata;
mod registry;

use std::process;

const APP_NAME: &str = "toolhub";
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        None | Some("gui") => gui::run(),
        Some("list") => list_tools(),
        Some("check") => check_tools(),
        Some("registry") => {
            let registry = registry::Registry::load();
            warn_problems(&registry);
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
    warn_problems(&registry);

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

/// 体检：版本比对 + 可执行文件 + 未注册的 workspace 包
///
/// 只读、软提示：不修改注册表，也不因发现问题而返回非零退出码。
fn check_tools() {
    let registry = registry::Registry::load();
    let config = config::HubConfig::load();
    let packages = match metadata::manifest_path() {
        Some(manifest) => match metadata::run(&manifest) {
            Ok(packages) => Some(packages),
            Err(e) => {
                eprintln!("提示：{}（版本比对与未注册发现已跳过）", e);
                None
            }
        },
        None => {
            eprintln!("提示：找不到 workspace 的 Cargo.toml（版本比对与未注册发现已跳过）");
            None
        }
    };

    let mut cache = launcher::ResolveCache::default();
    let health = health::collect(&registry, packages.as_ref(), &config, &mut cache);
    let total = registry.problems.len() + health.len();

    println!("注册表：{}", registry.source.label());
    println!(
        "工具 {} 个 · 体检问题 {} 条（版本不一致 / 可执行文件）",
        registry.tools.len(),
        total
    );
    for problem in registry.problems.iter().chain(health.iter()) {
        println!("  - {}", problem.display());
    }

    if let Some(packages) = &packages {
        let unregistered = health::unregistered(&registry, packages);
        if unregistered.is_empty() {
            println!("未注册的 workspace 包：无");
        } else {
            println!(
                "未注册的 workspace 包（{}）：{}",
                unregistered.len(),
                unregistered.join("、")
            );
            println!("  补登记见 toolhub/README.md「注册新工具」；toolhub 不会自动写入注册表。");
        }
    }

    if total == 0 {
        println!("体检通过：版本一致，可执行文件可解析");
    }
}

/// 注册表校验问题打印到 stderr（保持 stdout 仍是可直接解析的表格）
///
/// 与 GUI 一致：只做软提示，不影响其它工具的使用，也不把退出码改成失败。
fn warn_problems(registry: &registry::Registry) {
    if registry.problems.is_empty() {
        return;
    }
    eprintln!("警告：注册表有 {} 条问题：", registry.problems.len());
    for problem in &registry.problems {
        eprintln!("  - {}", problem.display());
    }
}

fn print_usage() {
    println!("{} v{} - Tao Tools 工具中心", APP_NAME, VERSION);
    println!();
    println!("用法：");
    println!("  {}                              打开工具中心界面", APP_NAME);
    println!("  {} list                         列出已注册工具", APP_NAME);
    println!("  {} check                        体检：版本 / 可执行文件 / 未注册包", APP_NAME);
    println!("  {} registry                     显示当前生效的注册表", APP_NAME);
    println!("  {} open-registry                打开注册表文件", APP_NAME);
    println!("  {} --version, -v                显示版本", APP_NAME);
    println!("  {} --help, -h                   显示帮助", APP_NAME);
    println!();
    println!("新增工具后必须注册到 toolhub/registry.json，详见 toolhub/README.md。");
}
