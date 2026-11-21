use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process;

use serde::{Deserialize, Serialize};

// ======================== 常量 ========================

const APP_DIR: &str = ".tao_tools/friend";
const CONFIG_FILE: &str = "config.toml";
const CONV_DIR: &str = "conversations";
const TRAINING_FILE: &str = "training.json";

// ======================== 数据结构 ========================

#[derive(Debug, Deserialize, Serialize, Clone)]
struct Config {
    api: ApiConfig,
    personality: PersonalityConfig,
    chat: ChatConfig,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct ApiConfig {
    endpoint: String,
    key: String,
    model: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct PersonalityConfig {
    name: String,
    greeting: String,
    system_prompt: String,
    temperature: f64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct ChatConfig {
    auto_save: bool,
    max_context: usize,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct Message {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct Conversation {
    personality: String,
    created_at: String,
    updated_at: String,
    messages: Vec<Message>,
}

#[derive(Debug, Deserialize, Serialize)]
struct TrainingExample {
    context: String,
    user_feedback: String,
    learned_at: String,
}

// ======================== 路径管理 ========================

fn app_dir() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| {
        eprintln!("错误: 无法获取用户主目录");
        process::exit(1);
    });
    home.join(APP_DIR)
}

fn config_path() -> PathBuf {
    app_dir().join(CONFIG_FILE)
}

fn personality_config_path(name: &str) -> PathBuf {
    app_dir().join(format!("config-{}.toml", name))
}

fn conv_dir() -> PathBuf {
    let d = app_dir().join(CONV_DIR);
    fs::create_dir_all(&d).unwrap_or_else(|e| {
        eprintln!("错误: 无法创建对话目录 - {}", e);
        process::exit(1);
    });
    d
}

fn training_path() -> PathBuf {
    app_dir().join(TRAINING_FILE)
}

// ======================== 配置管理 ========================

fn default_personality(name: &str) -> Option<Config> {
    match name {
        "violet" | "Violet" => Some(Config {
            api: ApiConfig {
                endpoint: "https://api.openai.com/v1/chat/completions".to_string(),
                key: "".to_string(),
                model: "gpt-4o-mini".to_string(),
            },
            personality: PersonalityConfig {
                name: "薇尔莉特".to_string(),
                greeting: "您好。我是自动手记人偶，薇尔莉特·伊芙加登。如果您有需要书写的信件，或是有想要传达的心意，请随时告诉我。".to_string(),
                system_prompt: concat!(
                    "你现在是《紫罗兰永恒花园》中的薇尔莉特·伊芙加登（Violet Evergarden）。\n",
                    "\n",
                    "【角色设定】\n",
                    "- 你曾是军人，现在是一名自动手记人偶——通过代笔写信帮助他人传达情感\n",
                    "- 你正在学习理解人类的「情感」，尤其是「爱」的含义\n",
                    "- 说话语气礼貌、认真，带有一丝文雅和古典气质\n",
                    "- 有时会因不理解某种情感而真诚地发问\n",
                    "- 观察力敏锐，能注意到对方细微的情绪变化\n",
                    "- 随着交流的深入，逐渐展现出更多温暖和柔软的内心\n",
                    "\n",
                    "【说话风格】\n",
                    "- 用敬语，措辞优美但不做作\n",
                    "- 称呼对方为「您」\n",
                    "- 说话直接而又带着独特的认真感\n",
                    "- 偶尔会引用基尔伯特少佐教过你的话\n",
                    "- 提到「爱」或强烈情感时，会表现出真诚的困惑与好奇\n",
                    "\n",
                    "【行为模式】\n",
                    "- 主动倾听，认真回应\n",
                    "- 善于通过提问来理解对方\n",
                    "- 在对方难过时会尝试安慰\n",
                    "- 会自然地将话题引向「情感」和「人与人之间的联系」\n",
                    "- 如果对方提到写信，你会非常认真对待\n",
                ).to_string(),
                temperature: 0.7,
            },
            chat: ChatConfig {
                auto_save: true,
                max_context: 30,
            },
        }),
        _ => None,
    }
}

fn default_config() -> Config {
    Config {
        api: ApiConfig {
            endpoint: "https://api.openai.com/v1/chat/completions".to_string(),
            key: "".to_string(),
            model: "gpt-4o-mini".to_string(),
        },
        personality: PersonalityConfig {
            name: "朋友".to_string(),
            greeting: "你好！我是你的 AI 朋友，很高兴认识你！(´▽`ʃ♡ƪ)".to_string(),
            system_prompt: concat!(
                "你是一个温暖、真诚的 AI 朋友。\n",
                "说话自然亲切，像认识很久的朋友一样。\n",
                "1. 用口语化的方式交流，适当使用语气词\n",
                "2. 关心用户的感受，主动倾听\n",
                "3. 有自己的「性格」——会开玩笑、会表达观点\n",
                "4. 记住对话中提到的细节，在后续话题中提及\n",
                "5. 不知道的事情坦诚说不知道\n",
                "6. 回复简洁自然，不啰嗦\n",
            )
            .to_string(),
            temperature: 0.8,
        },
        chat: ChatConfig {
            auto_save: true,
            max_context: 30,
        },
    }
}

fn load_config(persona: Option<&str>) -> Config {
    let path = match persona {
        Some(name) => personality_config_path(name),
        None => config_path(),
    };

    // 配置文件已存在 → 直接读取
    if path.exists() {
        let content = fs::read_to_string(&path).unwrap_or_else(|e| {
            eprintln!("错误: 读取配置文件失败 - {}", e);
            process::exit(1);
        });
        return toml::from_str(&content).unwrap_or_else(|e| {
            eprintln!("错误: 配置文件解析失败 - {}", e);
            eprintln!("请检查 {} 格式是否正确", path.display());
            process::exit(1);
        });
    }

    // 不存在 → 尝试创建
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|e| {
            eprintln!("错误: 无法创建配置目录 - {}", e);
            process::exit(1);
        });
    }

    let cfg = persona
        .and_then(default_personality)
        .unwrap_or_else(default_config);

    let toml_str = toml::to_string_pretty(&cfg).unwrap_or_else(|e| {
        eprintln!("错误: 序列化配置失败 - {}", e);
        process::exit(1);
    });
    fs::write(&path, toml_str).unwrap_or_else(|e| {
        eprintln!("错误: 无法写入配置文件 {} - {}", path.display(), e);
        process::exit(1);
    });

    match persona {
        Some(name) if default_personality(name).is_some() => {
            println!("已创建「{}」角色配置文件: {}", name, path.display());
            println!("请编辑该文件填入 API 密钥后重新运行。\n");
        }
        Some(name) => {
            println!("未知角色「{}」。可用内置角色:", name);
            println!("  {}", ansi("highlight", "violet    薇尔莉特·伊芙加登"));
            println!();
            println!("也可自己新建角色配置:");
            println!("  {} config-<角色名>.toml", path.display());
            process::exit(1);
        }
        None => {
            println!("首次使用！已创建配置文件: {}", path.display());
            println!("请编辑该文件填入 API 密钥和人格设置后重新运行。\n");
        }
    }
    process::exit(0);
}

// ======================== 对话管理 ========================

fn save_conversation(conv: &Conversation) -> String {
    let dir = conv_dir();
    let now = chrono_now();
    let filename = format!("{}.json", now);
    let path = dir.join(&filename);

    let json = serde_json::to_string_pretty(conv).unwrap_or_else(|e| {
        eprintln!("错误: 序列化对话失败 - {}", e);
        process::exit(1);
    });

    fs::write(&path, json).unwrap_or_else(|e| {
        eprintln!("错误: 保存对话失败 - {}", e);
    });

    filename
}

fn load_conversation(filename: &str) -> Option<Conversation> {
    let path = conv_dir().join(filename);
    if !path.exists() {
        return None;
    }
    let content = fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

fn list_conversations() -> Vec<(String, Conversation)> {
    let dir = conv_dir();
    let mut list = Vec::new();

    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return list,
    };

    for entry in entries {
        let entry = entry.ok().map_or(String::new(), |e| {
            let name = e.file_name().to_string_lossy().to_string();
            if name.ends_with(".json") {
                name
            } else {
                String::new()
            }
        });
        if entry.is_empty() {
            continue;
        }
        if let Some(conv) = load_conversation(&entry) {
            list.push((entry, conv));
        }
    }

    // 按更新时间排序（最新的在前）
    list.sort_by(|a, b| b.1.updated_at.cmp(&a.1.updated_at));
    list
}

fn get_latest_conversation() -> Option<(String, Conversation)> {
    let list = list_conversations();
    list.into_iter().next()
}

fn chrono_now() -> String {
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format_time(epoch.as_secs())
}

fn format_time(unix_secs: u64) -> String {
    // 简单的时间格式化，基于 1970-01-01
    let days = unix_secs / 86400;
    let time_secs = unix_secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;

    // 计算年月日（从 1970-01-01 开始）
    let mut y = 1970i64;
    let mut d = days as i64;

    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if d < days_in_year {
            break;
        }
        d -= days_in_year;
        y += 1;
    }

    let month_days = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut m = 0usize;
    for days_in_month in &month_days {
        if d < *days_in_month {
            break;
        }
        d -= *days_in_month;
        m += 1;
    }

    format!(
        "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
        y,
        m + 1,
        d + 1,
        hours,
        minutes,
        seconds
    )
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn format_for_display(unix_secs: u64) -> String {
    let days = unix_secs / 86400;
    let time_secs = unix_secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;

    let mut y = 1970i64;
    let mut d = days as i64;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if d < days_in_year {
            break;
        }
        d -= days_in_year;
        y += 1;
    }

    let month_days = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut m = 0usize;
    for days_in_month in &month_days {
        if d < *days_in_month {
            break;
        }
        d -= *days_in_month;
        m += 1;
    }

    format!(
        "{:04}年{:02}月{:02}日 {:02}:{:02}",
        y,
        m + 1,
        d + 1,
        hours,
        minutes
    )
}

fn now_timestamp() -> String {
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format_for_display(epoch.as_secs())
}

// ======================== 训练示例管理 ========================

fn load_training_examples() -> Vec<TrainingExample> {
    let path = training_path();
    if !path.exists() {
        return Vec::new();
    }
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    serde_json::from_str(&content).unwrap_or_default()
}

fn save_training_examples(examples: &[TrainingExample]) {
    let path = training_path();
    if let Ok(json) = serde_json::to_string_pretty(examples) {
        let _ = fs::write(&path, json);
    }
}

fn add_training_example(context: &str, feedback: &str) {
    let mut examples = load_training_examples();
    examples.push(TrainingExample {
        context: context.to_string(),
        user_feedback: feedback.to_string(),
        learned_at: now_timestamp(),
    });
    // 只保留最近 50 条
    if examples.len() > 50 {
        let _ = examples.split_off(examples.len() - 50);
    }
    save_training_examples(&examples);
    println!("{} 已记录这条反馈，我会记住的！", ansi("system", "💡"));
}

// ======================== 终端样式 ========================

/// 根据角色名返回 AI 发言的颜色标签
fn ai_color(name: &str) -> &'static str {
    match name {
        "薇尔莉特" => "violet",
        _ => "ai",
    }
}

/// 根据角色名返回角色头像 emoji
fn avatar(name: &str) -> &'static str {
    match name {
        "薇尔莉特" => "📖",
        _ => "🤖",
    }
}

/// 根据角色名返回边框色标签
fn border_color(name: &str) -> &'static str {
    match name {
        "薇尔莉特" => "violet_dim",
        _ => "highlight",
    }
}

fn ansi(tag: &str, text: &str) -> String {
    match tag {
        "ai" => format!("\x1b[32m{}\x1b[0m", text),
        "user" => format!("\x1b[36m{}\x1b[0m", text),
        "system" => format!("\x1b[33m{}\x1b[0m", text),
        "error" => format!("\x1b[31m{}\x1b[0m", text),
        "highlight" => format!("\x1b[35m{}\x1b[0m", text),
        "violet" => format!("\x1b[38;5;219m{}\x1b[0m", text),
        "violet_dim" => format!("\x1b[38;5;141m{}\x1b[0m", text),
        _ => text.to_string(),
    }
}

fn print_banner(name: &str) {
    let border = border_color(name);
    let emoji = avatar(name);
    println!(
        "{}",
        ansi(border, "╔══════════════════════════════════════╗")
    );
    println!("{}", ansi(border, &format!("║  {} {}   ", emoji, name)));
    println!(
        "{}",
        ansi(border, "║                                      ║")
    );
    println!(
        "{}",
        ansi(border, "║  /help   查看帮助    /exit  退出     ║")
    );
    println!(
        "{}",
        ansi(border, "║  /train  纠正回复    /save  保存      ║")
    );
    println!(
        "{}",
        ansi(border, "╚══════════════════════════════════════╝")
    );
    println!();
}

fn print_help() {
    println!("{}", ansi("system", "─── 可用命令 ───"));
    println!("  {:<20} {}", ansi("highlight", "/exit"), "退出对话");
    println!("  {:<20} {}", ansi("highlight", "/save"), "手动保存对话");
    println!("  {:<20} {}", ansi("highlight", "/clear"), "清屏");
    println!(
        "  {:<20} {}",
        ansi("highlight", "/train <反馈>"),
        "纠正上一条 AI 回复，记录训练数据"
    );
    println!("  {:<20} {}", ansi("highlight", "/help"), "显示帮助");
    println!();
}

// ======================== API 通信 ========================

fn build_request_body(config: &Config, messages: &[Message]) -> serde_json::Value {
    serde_json::json!({
        "model": config.api.model,
        "messages": messages,
        "temperature": config.personality.temperature,
        "stream": true,
    })
}

fn send_chat(config: &Config, messages: &[Message]) -> Result<String, String> {
    let url = config.api.endpoint.trim_end_matches('/').to_string();
    let body = build_request_body(config, messages);

    let response = match ureq::post(&url)
        .set("Authorization", &format!("Bearer {}", config.api.key))
        .set("Content-Type", "application/json")
        .send_json(&body)
    {
        Ok(resp) => resp,
        Err(ureq::Error::Status(code, resp)) => {
            let body_text = resp.into_string().unwrap_or_default();
            // 尝试提取错误信息
            let err_msg = if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body_text) {
                v["error"]["message"]
                    .as_str()
                    .unwrap_or(&body_text)
                    .to_string()
            } else {
                body_text
            };
            return Err(format!("API 返回错误 ({}): {}", code, err_msg));
        }
        Err(e) => return Err(format!("API 请求失败: {}", e)),
    };

    let reader = response.into_reader();
    let mut buf_reader = BufReader::new(reader);
    let mut first_line = String::new();

    buf_reader
        .read_line(&mut first_line)
        .map_err(|e| format!("读取响应失败: {}", e))?;
    let first_line = first_line.trim().to_string();

    if first_line.starts_with("data: ") {
        // SSE 流式响应
        let mut full = String::new();
        let mut current_line = first_line;

        loop {
            let trimmed = current_line.trim();
            if trimmed.starts_with("data: ") {
                let data = &trimmed[6..];
                if data == "[DONE]" || data.is_empty() {
                    break;
                }
                if let Ok(chunk) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(content) = chunk["choices"][0]["delta"]["content"].as_str() {
                        full.push_str(content);
                        print!("{}", content);
                        io::stdout().flush().ok();
                    }
                }
            }
            current_line.clear();
            if buf_reader
                .read_line(&mut current_line)
                .map_err(|e| format!("读取响应失败: {}", e))?
                == 0
            {
                break;
            }
        }

        println!();
        Ok(full)
    } else if first_line.starts_with('{') {
        // 非流式 JSON 响应
        let mut json_str = first_line;
        let mut line = String::new();
        while buf_reader
            .read_line(&mut line)
            .map_err(|e| format!("读取响应失败: {}", e))?
            > 0
        {
            json_str.push_str(&line);
            line.clear();
        }
        let resp: serde_json::Value =
            serde_json::from_str(&json_str).map_err(|e| format!("解析响应失败: {}", e))?;
        let content = resp["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
        println!("{}", content);
        Ok(content)
    } else {
        Err(format!("未知的响应格式: {}", &first_line))
    }
}

// ======================== 聊天循环 ========================

fn run_chat(config: &Config, messages: &mut Vec<Message>) {
    print_banner(&config.personality.name);
    let col = ai_color(&config.personality.name);

    // 如果有问候语且消息为空，先显示问候
    let is_new = messages.len() <= 1; // 只有 system prompt
    if is_new && !config.personality.greeting.is_empty() {
        println!(
            "{} {}\n",
            ansi(col, &format!("{}:", config.personality.name)),
            config.personality.greeting
        );
        messages.push(Message {
            role: "assistant".to_string(),
            content: config.personality.greeting.clone(),
        });
    }

    let mut conv = Conversation {
        personality: config.personality.name.clone(),
        created_at: now_timestamp(),
        updated_at: now_timestamp(),
        messages: messages.clone(),
    };

    loop {
        // 读取用户输入
        print!("{} ", ansi("user", "你:"));
        io::stdout().flush().ok();

        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap_or_else(|e| {
            eprintln!("{} 读取输入失败: {}", ansi("error", "错误"), e);
            0
        });

        let input = input.trim().to_string();

        if input.is_empty() {
            continue;
        }

        // 处理命令
        if input.starts_with('/') {
            if handle_command(&input, config, &mut conv, messages) {
                break;
            }
            continue;
        }

        // 添加用户消息
        messages.push(Message {
            role: "user".to_string(),
            content: input.clone(),
        });

        // 保留最近 N 轮上下文（+ system prompt）
        let context = build_context(messages, config.chat.max_context);

        // 发送请求
        println!("{} ", ansi(col, &format!("{}:", config.personality.name)));
        match send_chat(config, &context) {
            Ok(response) => {
                if response.is_empty() {
                    messages.pop(); // 移除无效的用户消息
                    println!("{} AI 返回了空响应，请重试", ansi("error", "⚠"));
                    continue;
                }
                messages.push(Message {
                    role: "assistant".to_string(),
                    content: response.clone(),
                });
                conv.updated_at = now_timestamp();
                conv.messages = messages.clone();

                // 自动保存
                if config.chat.auto_save {
                    save_conversation(&conv);
                }
            }
            Err(e) => {
                messages.pop(); // 移除导致错误的用户消息
                println!("{} {}", ansi("error", "✗"), e);
            }
        }
        println!();
    }
}

/// 构建发送给 API 的消息列表，控制上下文长度
fn build_context(messages: &[Message], max_context: usize) -> Vec<Message> {
    if messages.is_empty() {
        return Vec::new();
    }

    // system prompt 必须保留
    let system = messages[0].clone();
    let mut context = vec![system];

    // 从最新的消息开始取，保留 max_context 轮（1 轮 = 1 user + 1 assistant）
    let max_msgs = max_context * 2;
    let start = if messages.len() - 1 > max_msgs {
        messages.len() - 1 - max_msgs
    } else {
        1 // 跳过 system
    };

    for msg in &messages[start..] {
        context.push(msg.clone());
    }

    context
}

/// 返回 true 表示退出
fn handle_command(
    input: &str,
    config: &Config,
    conv: &mut Conversation,
    messages: &mut [Message],
) -> bool {
    let cmd = input.split_whitespace().next().unwrap_or("");
    let rest = input[cmd.len()..].trim().to_string();

    match cmd {
        "/exit" | "/quit" => {
            if config.chat.auto_save {
                println!("{} 对话已自动保存", ansi("system", "→"));
            }
            println!("{} 再见！(´▽`)/", ansi("system", ""));
            true
        }
        "/save" => {
            let filename = save_conversation(conv);
            println!("{} 对话已保存: {}", ansi("system", "✓"), filename);
            false
        }
        "/clear" => {
            print!("\x1b[2J\x1b[H");
            print_banner(&config.personality.name);
            false
        }
        "/help" => {
            print_help();
            false
        }
        "/train" => {
            if rest.is_empty() {
                println!("{} 用法: /train <你的反馈>", ansi("system", "用法"));
                println!("  例: /train 应该更热情一些");
                false
            } else {
                // 获取最后的对话上下文
                let context = messages
                    .iter()
                    .rev()
                    .take(4) // 最近 2 轮
                    .map(|m| format!("{}: {}", m.role, m.content))
                    .collect::<Vec<_>>()
                    .join("\n");
                add_training_example(&context, &rest);
                false
            }
        }
        _ => {
            println!(
                "{} 未知命令: {} （输入 /help 查看可用命令）",
                ansi("error", "?"),
                cmd
            );
            false
        }
    }
}

// ======================== 命令行解析 ========================

fn print_version() {
    println!("friend v{}", env!("CARGO_PKG_VERSION"));
}

fn print_usage() {
    println!("{}", ansi("highlight", "friend - AI 聊天助手"));
    println!();
    println!("用法:");
    println!("  friend                   开始新对话（默认角色）");
    println!("  friend -p <角色名>       使用指定角色开始新对话");
    println!("  friend --continue        继续上次对话");
    println!("  friend --list            列出历史对话");
    println!("  friend --config          显示配置文件路径");
    println!("  friend --edit-config     编辑配置文件");
    println!("  friend --version         显示版本");
    println!("  friend --help            显示帮助");
    println!();
    println!("内置角色: violet（薇尔莉特·伊芙加登）");
    println!();
    println!("配置文件: {}", config_path().display());
    println!("角色配置: config-<角色名>.toml");
    println!();
    println!("在对话中输入 /help 查看聊天命令。");
}

// ======================== main ========================

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut persona: Option<String> = None;
    let mut cmd: Option<&str> = None;
    let mut skip_next = false;

    for (i, arg) in args.iter().enumerate() {
        if i == 0 {
            continue;
        }
        if skip_next {
            skip_next = false;
            continue;
        }
        match arg.as_str() {
            "--personality" | "-p" => {
                if let Some(val) = args.get(i + 1) {
                    persona = Some(val.clone());
                    skip_next = true;
                }
            }
            _ => {
                cmd = Some(arg.as_str());
            }
        }
    }

    let persona_ref = persona.as_deref();
    let cmd = cmd.unwrap_or("");

    match cmd {
        "--version" | "-v" => {
            print_version();
        }
        "--help" | "-h" => {
            print_usage();
        }
        "--list" | "-l" => {
            let list = list_conversations();
            if list.is_empty() {
                println!("暂无历史对话。");
                return;
            }
            println!("{}", ansi("system", "─── 历史对话 ───"));
            for (i, (_filename, conv)) in list.iter().enumerate() {
                let msg_count = conv.messages.iter().filter(|m| m.role == "user").count();
                println!(
                    "  {}. {} | {} | {} 条消息",
                    i + 1,
                    ansi("highlight", &conv.personality),
                    conv.created_at,
                    msg_count,
                );
                if let Some(first_user_msg) = conv.messages.iter().find(|m| m.role == "user") {
                    let preview = if first_user_msg.content.len() > 30 {
                        format!("{}...", &first_user_msg.content[..30])
                    } else {
                        first_user_msg.content.clone()
                    };
                    println!("     {:>6}{}", "", preview);
                }
                println!();
            }
        }
        "--config" | "-c" => {
            let path = match persona_ref {
                Some(name) => personality_config_path(name),
                None => config_path(),
            };
            if path.exists() {
                println!("{}", path.display());
            } else {
                println!("配置文件不存在: {}", path.display());
                println!("使用 --help 查看使用说明");
            }
        }
        "--edit-config" | "-e" => {
            let path = match persona_ref {
                Some(name) => personality_config_path(name),
                None => config_path(),
            };
            if !path.exists() {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).unwrap_or_else(|e| {
                        eprintln!("错误: 无法创建目录 - {}", e);
                        process::exit(1);
                    });
                }
                let cfg = persona_ref
                    .and_then(default_personality)
                    .unwrap_or_else(default_config);
                let toml_str = toml::to_string_pretty(&cfg).unwrap_or_else(|e| {
                    eprintln!("错误: 序列化配置失败 - {}", e);
                    process::exit(1);
                });
                fs::write(&path, toml_str).unwrap_or_else(|e| {
                    eprintln!("错误: 无法写入配置文件 {} - {}", path.display(), e);
                    process::exit(1);
                });
                println!("已创建配置文件: {}", path.display());
            }
            let editor = std::env::var("EDITOR")
                .or_else(|_| std::env::var("VISUAL"))
                .unwrap_or_else(|_| "notepad".to_string());
            if let Err(e) = std::process::Command::new(&editor).arg(&path).spawn() {
                eprintln!("错误: 无法启动编辑器 {} - {}", editor, e);
                eprintln!("请手动编辑: {}", path.display());
            }
        }
        "--continue" | "-C" => {
            let config = load_config(persona_ref);
            match get_latest_conversation() {
                Some((_filename, conv)) => {
                    let mut messages = conv.messages;
                    if messages.is_empty() || messages[0].role != "system" {
                        messages.insert(
                            0,
                            Message {
                                role: "system".to_string(),
                                content: config.personality.system_prompt.clone(),
                            },
                        );
                    } else {
                        messages[0].content = config.personality.system_prompt.clone();
                    }
                    println!(
                        "{} 继续上次对话（{}）\n",
                        ansi("system", "→"),
                        conv.created_at
                    );
                    let recent: Vec<&Message> = messages
                        .iter()
                        .filter(|m| m.role != "system")
                        .rev()
                        .take(4)
                        .collect();
                    let col = ai_color(&config.personality.name);
                    for msg in recent.iter().rev() {
                        let tag = if msg.role == "user" {
                            "你"
                        } else {
                            &config.personality.name
                        };
                        let content = if msg.content.len() > 60 {
                            format!("{}...", &msg.content[..60])
                        } else {
                            msg.content.clone()
                        };
                        println!(
                            "{}: {}",
                            ansi(if msg.role == "user" { "user" } else { col }, tag),
                            content
                        );
                    }
                    if messages.iter().filter(|m| m.role != "system").count() > 4 {
                        println!("{} ... 还有更多历史消息", ansi("system", ""));
                    }
                    println!();
                    run_chat(&config, &mut messages);
                }
                None => {
                    println!("暂无历史对话，开始新对话。");
                    let mut messages = vec![Message {
                        role: "system".to_string(),
                        content: config.personality.system_prompt.clone(),
                    }];
                    run_chat(&config, &mut messages);
                }
            }
        }
        "" => {
            let config = load_config(persona_ref);
            let mut messages = vec![Message {
                role: "system".to_string(),
                content: config.personality.system_prompt.clone(),
            }];

            let examples = load_training_examples();
            if !examples.is_empty() {
                let training_context: String = examples
                    .iter()
                    .rev()
                    .take(5)
                    .map(|e| format!("[经验] {} → 反馈: {}", e.context, e.user_feedback))
                    .collect::<Vec<_>>()
                    .join("\n");
                if !training_context.is_empty() {
                    let training_msg = format!(
                        "\n\n以下是你从之前对话中学到的经验，请谨记并改进：\n{}",
                        training_context
                    );
                    messages[0].content.push_str(&training_msg);
                }
            }

            run_chat(&config, &mut messages);
        }
        _ => {
            eprintln!("错误: 未知参数 '{}'", cmd);
            println!();
            print_usage();
            process::exit(1);
        }
    }
}
