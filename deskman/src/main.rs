mod config;
mod desktop;
mod desktop_list;
mod gui;
mod tools;
mod tray;
mod window;

use std::process;

const APP_NAME: &str = "deskman";
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        print_usage();
        return;
    }

    match args[1].as_str() {
        "gui" => gui::run(),
        "config" => println!("{}", config::config_path().display()),
        "scan" => handle_scan(),
        "list-desktop" => handle_list_desktop(),
        "move-icon" => handle_move_icon(&args[2..]),
        "restore-positions" => handle_restore_positions(&args[2..]),
        "toggle-icons" => {
            match desktop_list::toggle_view() {
                Ok(()) => println!("已切换"),
                Err(e) => println!("失败: {}", e),
            }
        }
        "window" => handle_window(&args[2..]),
        "tools" => handle_tools(&args[2..]),
        "--help" | "-h" => print_usage(),
        "--version" | "-v" => println!("{} v{}", APP_NAME, VERSION),
        _ => {
            eprintln!("错误：未知命令 '{}'", args[1]);
            print_usage();
            process::exit(1);
        }
    }
}

fn handle_window(args: &[String]) {
    if args.is_empty() {
        print_window_usage();
        return;
    }

    match args[0].as_str() {
        "list" => {
            let windows = window::list_windows().unwrap_or_else(exit_with_error);
            if windows.is_empty() {
                println!("未检测到可管理窗口。");
                return;
            }
            println!(
                "{:<10} {:<8} {:<6} {:<20} {}",
                "句柄", "PID", "状态", "位置", "标题"
            );
            for item in windows {
                println!(
                    "{:<10} {:<8} {:<6} {:<20} {}",
                    item.handle,
                    item.pid,
                    if item.minimized {
                        "最小化"
                    } else {
                        "正常"
                    },
                    item.rect_label(),
                    item.title
                );
            }
        }
        "find" => {
            let keyword = required_arg(args, 1, "缺少窗口关键词");
            let windows = window::find_windows(keyword).unwrap_or_else(exit_with_error);
            for item in windows {
                println!(
                    "{} | PID {} | {} | {}",
                    item.handle,
                    item.pid,
                    item.rect_label(),
                    item.title
                );
            }
        }
        "focus" => {
            let keyword = required_arg(args, 1, "缺少窗口关键词");
            let item = window::focus_window(keyword).unwrap_or_else(exit_with_error);
            println!("已聚焦窗口：{} ({})", item.title, item.handle);
        }
        "move" => {
            if args.len() < 6 {
                eprintln!("错误：缺少参数");
                eprintln!("用法：{} window move <关键词> <x> <y> <w> <h>", APP_NAME);
                process::exit(1);
            }
            let keyword = &args[1];
            let x = parse_i32(&args[2], "x");
            let y = parse_i32(&args[3], "y");
            let w = parse_i32(&args[4], "w");
            let h = parse_i32(&args[5], "h");
            let item = window::move_window(keyword, x, y, w, h).unwrap_or_else(exit_with_error);
            println!("已移动窗口：{} -> {},{},{},{}", item.title, x, y, w, h);
        }
        "topmost" => {
            if args.len() < 3 {
                eprintln!("错误：缺少参数");
                eprintln!("用法：{} window topmost <关键词> on|off", APP_NAME);
                process::exit(1);
            }
            let enable = match args[2].as_str() {
                "on" => true,
                "off" => false,
                other => {
                    eprintln!("错误：topmost 只支持 on 或 off，收到 '{}'", other);
                    process::exit(1);
                }
            };
            let item = window::set_topmost(&args[1], enable).unwrap_or_else(exit_with_error);
            println!(
                "已{}置顶：{} ({})",
                if enable { "" } else { "取消" },
                item.title,
                item.handle
            );
        }
        "minimize" => {
            let keyword = required_arg(args, 1, "缺少窗口关键词");
            let item = window::minimize_window(keyword).unwrap_or_else(exit_with_error);
            println!("已最小化窗口：{} ({})", item.title, item.handle);
        }
        "restore" => {
            let keyword = required_arg(args, 1, "缺少窗口关键词");
            let item = window::restore_window(keyword).unwrap_or_else(exit_with_error);
            println!("已恢复窗口：{} ({})", item.title, item.handle);
        }
        "--help" | "-h" => print_window_usage(),
        _ => {
            eprintln!("错误：未知 window 命令 '{}'", args[0]);
            print_window_usage();
            process::exit(1);
        }
    }
}

fn handle_tools(args: &[String]) {
    if args.is_empty() {
        print_tools_usage();
        return;
    }

    match args[0].as_str() {
        "list" => {
            let list = tools::list_tools().unwrap_or_else(exit_with_error);
            if list.is_empty() {
                println!("未找到 tao_tools 工具。");
                return;
            }
            println!("{:<12} {:<10} {}", "工具", "版本", "简介");
            for tool in list {
                println!(
                    "{:<12} v{:<8} {}",
                    tool.name, tool.version, tool.description
                );
            }
        }
        "build" => {
            let name = required_arg(args, 1, "缺少工具名");
            tools::run_cargo_for_tool("build", name).unwrap_or_else(exit_with_error);
        }
        "install" => {
            let name = required_arg(args, 1, "缺少工具名");
            tools::install_tool(name).unwrap_or_else(exit_with_error);
        }
        "readme" => {
            let name = required_arg(args, 1, "缺少工具名");
            let path = tools::tool_readme_path(name).unwrap_or_else(exit_with_error);
            println!("{}", path.display());
        }
        "--help" | "-h" => print_tools_usage(),
        _ => {
            eprintln!("错误：未知 tools 命令 '{}'", args[0]);
            print_tools_usage();
            process::exit(1);
        }
    }
}

fn print_usage() {
    println!("{} v{} - 桌面快捷方式分类整理器", APP_NAME, VERSION);
    println!();
    println!("用法：");
    println!(
        "  {} gui                              启动桌面常驻分类界面",
        APP_NAME
    );
    println!(
        "  {} config                           显示配置文件路径",
        APP_NAME
    );
    println!(
        "  {} scan                             扫描桌面快捷方式和文件",
        APP_NAME
    );
    println!(
        "  {} window <命令>                    高级窗口命令",
        APP_NAME
    );
    println!(
        "  {} tools <命令>                     管理 tao_tools 工具",
        APP_NAME
    );
    println!("  {} --version, -v                    显示版本", APP_NAME);
    println!("  {} --help, -h                       显示帮助", APP_NAME);
    println!();
    print_window_usage();
    println!();
    print_tools_usage();
}

/// 一次性：按状态文件记录把桌面图标放回原位
fn handle_restore_positions(args: &[String]) {
    let path = if args.is_empty() {
        crate::config::desktop_state_path()
    } else {
        std::path::PathBuf::from(&args[0])
    };
    let Ok(content) = std::fs::read_to_string(&path) else {
        eprintln!("无法读取状态文件 {}", path.display());
        std::process::exit(1);
    };
    #[derive(serde::Deserialize)]
    struct Taken {
        name: String,
        x: i32,
        y: i32,
    }
    #[derive(serde::Deserialize)]
    struct State {
        taken: Vec<Taken>,
    }
    let state: State = serde_json::from_str(&content).expect("状态文件解析失败");
    let icons = desktop_list::read_desktop_icons();
    let mut restored = 0;
    let mut used = std::collections::HashSet::new();
    for taken in &state.taken {
        if let Some(icon) = icons
            .iter()
            .find(|i| i.name == taken.name && !used.contains(&i.index))
        {
            if desktop_list::set_icon_pos(icon.index, taken.x, taken.y) {
                used.insert(icon.index);
                restored += 1;
            }
        }
    }
    println!("已恢复 {} 个图标位置", restored);
}

fn handle_move_icon(args: &[String]) {
    if args.len() < 3 {
        eprintln!("用法: deskman move-icon <index> <x> <y>");
        std::process::exit(1);
    }
    let index: i32 = args[0].parse().unwrap();
    let x: i32 = args[1].parse().unwrap();
    let y: i32 = args[2].parse().unwrap();
    let ok16 = desktop_list::set_icon_pos(index, x, y);
    let ok32 = desktop_list::set_icon_pos32(index, x, y);
    println!("packed={ok16} ptr32={ok32}");
    let icons = desktop_list::read_desktop_icons();
    if let Some(icon) = icons.iter().find(|i| i.index == index) {
        println!("readback: ({}, {}) {}", icon.x, icon.y, icon.name);
    }
}

fn handle_list_desktop() {
    println!("自动排列: {}", desktop_list::auto_arrange_enabled());
    println!("系统图标尺寸(逻辑px): {:?}", desktop_list::system_icon_size_logical());
    let icons = desktop_list::read_desktop_icons();
    println!("桌面图标共 {} 个", icons.len());
    for icon in icons.iter().take(12) {
        println!(
            "  [{:>3}] ({:>6},{:>6}) {}",
            icon.index, icon.x, icon.y, icon.name
        );
    }
}

fn handle_scan() {
    let mut config = config::AppConfig::load();
    let count = desktop::scan_into_config(&mut config).unwrap_or_else(exit_with_error);
    config.save();
    println!("已扫描并导入 {} 个桌面项目", count);
    println!("配置文件：{}", config::config_path().display());
}

fn print_window_usage() {
    println!("高级窗口命令：");
    println!(
        "  {} window list                      列出可见窗口",
        APP_NAME
    );
    println!("  {} window find <关键词>             查找窗口", APP_NAME);
    println!("  {} window focus <关键词>            聚焦窗口", APP_NAME);
    println!(
        "  {} window move <关键词> <x> <y> <w> <h>  移动并调整窗口",
        APP_NAME
    );
    println!(
        "  {} window topmost <关键词> on|off   设置或取消置顶",
        APP_NAME
    );
    println!("  {} window minimize <关键词>         最小化窗口", APP_NAME);
    println!("  {} window restore <关键词>          恢复窗口", APP_NAME);
}

fn print_tools_usage() {
    println!("工具命令：");
    println!("  {} tools list                       列出工具", APP_NAME);
    println!(
        "  {} tools build <工具名>             构建指定工具",
        APP_NAME
    );
    println!(
        "  {} tools install <工具名>           安装指定工具到 cargo bin",
        APP_NAME
    );
    println!(
        "  {} tools readme <工具名>            输出 README 路径",
        APP_NAME
    );
}

fn required_arg<'a>(args: &'a [String], index: usize, message: &str) -> &'a str {
    args.get(index).map(String::as_str).unwrap_or_else(|| {
        eprintln!("错误：{}", message);
        process::exit(1);
    })
}

fn parse_i32(value: &str, name: &str) -> i32 {
    value.parse::<i32>().unwrap_or_else(|_| {
        eprintln!("错误：{} 必须是整数，收到 '{}'", name, value);
        process::exit(1);
    })
}

fn exit_with_error<T>(message: String) -> T {
    eprintln!("错误：{}", message);
    process::exit(1);
}
