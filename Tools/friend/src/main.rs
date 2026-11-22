use std::collections::HashSet;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::time::Duration;

use serde::{Deserialize, Serialize};

// ======================== 常量 ========================

const APP_DIR: &str = ".tao_tools/friend";
const CONFIG_FILE: &str = "config.toml";
const CONV_DIR: &str = "conversations";
const TRAINING_FILE: &str = "training.json";

/// 建立连接的超时（秒）：只连不回的端口最多等这么久
const CONNECT_TIMEOUT_SECS: u64 = 10;
/// 单次读取的超时（秒）：流式输出时超过这么久没有新数据就报超时
const READ_TIMEOUT_SECS: u64 = 60;

const DEFAULT_MODEL: &str = "gpt-4o-mini";
const DEFAULT_TEMPERATURE: f64 = 0.8;
const DEFAULT_MAX_CONTEXT: usize = 30;

// ======================== 数据结构 ========================

#[derive(Debug, Deserialize, Serialize, Clone)]
struct Config {
    api: ApiConfig,
    /// 整段 [personality] 缺失时用内置默认人格，不报错
    #[serde(default)]
    personality: PersonalityConfig,
    /// 整段 [chat] 缺失时用默认值（auto_save = true、max_context = 30）
    #[serde(default)]
    chat: ChatConfig,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct ApiConfig {
    endpoint: String,
    /// 留空也能解析成功，启动校验时给中文提示（而不是等到 401）
    #[serde(default)]
    key: String,
    #[serde(default = "default_model")]
    model: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct PersonalityConfig {
    name: String,
    /// 可留空：留空就不打印开场白
    #[serde(default)]
    greeting: String,
    #[serde(default = "default_system_prompt")]
    system_prompt: String,
    #[serde(default = "default_temperature")]
    temperature: f64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct ChatConfig {
    #[serde(default = "default_auto_save")]
    auto_save: bool,
    #[serde(default = "default_max_context")]
    max_context: usize,
}

impl Default for ChatConfig {
    fn default() -> Self {
        ChatConfig {
            auto_save: default_auto_save(),
            max_context: default_max_context(),
        }
    }
}

impl Default for PersonalityConfig {
    fn default() -> Self {
        PersonalityConfig {
            name: "朋友".to_string(),
            greeting: String::new(),
            system_prompt: default_system_prompt(),
            temperature: default_temperature(),
        }
    }
}

fn default_model() -> String {
    DEFAULT_MODEL.to_string()
}

fn default_temperature() -> f64 {
    DEFAULT_TEMPERATURE
}

fn default_auto_save() -> bool {
    true
}

fn default_max_context() -> usize {
    DEFAULT_MAX_CONTEXT
}


#[derive(Debug, Deserialize, Serialize, Clone)]
struct Message {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct Conversation {
    /// 会话唯一 id；v1.0.0 的旧文件没有该字段，读取时用文件名补齐
    #[serde(default)]
    id: String,
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
            chat: ChatConfig::default(),
        }),
        _ => None,
    }
}

/// 默认人格的系统提示词（内置人格与「[personality] 整段缺失」共用）
fn default_system_prompt() -> String {
    concat!(
        "你是一个温暖、真诚的 AI 朋友。\n",
        "说话自然亲切，像认识很久的朋友一样。\n",
        "1. 用口语化的方式交流，适当使用语气词\n",
        "2. 关心用户的感受，主动倾听\n",
        "3. 有自己的「性格」——会开玩笑、会表达观点\n",
        "4. 记住对话中提到的细节，在后续话题中提及\n",
        "5. 不知道的事情坦诚说不知道\n",
        "6. 回复简洁自然，不啰嗦\n",
    )
    .to_string()
}

fn default_config() -> Config {
    Config {
        api: ApiConfig {
            endpoint: "https://api.openai.com/v1/chat/completions".to_string(),
            key: "".to_string(),
            model: DEFAULT_MODEL.to_string(),
        },
        personality: PersonalityConfig {
            name: "朋友".to_string(),
            greeting: "你好！我是你的 AI 朋友，很高兴认识你！(´▽`ʃ♡ƪ)".to_string(),
            system_prompt: default_system_prompt(),
            temperature: DEFAULT_TEMPERATURE,
        },
        chat: ChatConfig::default(),
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
        return match toml::from_str::<Config>(&content) {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("{} 配置文件解析失败", ansi("error", "错误:"));
                for line in format_config_error(&content, &e, &path) {
                    eprintln!("{}", line);
                }
                process::exit(1);
            }
        };
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
    write_atomic(&path, &toml_str).unwrap_or_else(|e| {
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

/// 本次要用的配置文件路径（带角色时是 config-<角色>.toml）
fn config_file_for(persona: Option<&str>) -> PathBuf {
    match persona {
        Some(name) => personality_config_path(name),
        None => config_path(),
    }
}

/// 临时文件名：config.toml.tmp（与目标同目录，改名是原子的）
fn tmp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| CONFIG_FILE.to_string());
    path.with_file_name(format!("{}.tmp", name))
}

/// 「临时文件 + 改名替换」：写一半崩掉也不会把原配置写成半截内容
fn write_atomic(path: &Path, content: &str) -> io::Result<()> {
    let tmp = tmp_path(path);
    fs::write(&tmp, content)?;
    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// 字节偏移 → (行号, 列号)，都从 1 开始
fn line_col(content: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(content.len());
    let mut line = 1usize;
    let mut col = 1usize;
    for (i, ch) in content.char_indices() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// 把 toml 解析错误整理成能直接看懂的几行：错在第几行、缺哪个字段
fn format_config_error(content: &str, err: &toml::de::Error, path: &Path) -> Vec<String> {
    let mut out = vec![format!("  原因: {}", err.message())];
    if let Some(span) = err.span() {
        let (line_no, col) = line_col(content, span.start);
        out.push(format!(
            "  位置: {} 第 {} 行第 {} 列",
            path.display(),
            line_no,
            col
        ));
        if let Some(text) = content.lines().nth(line_no - 1) {
            out.push(format!("  > {}", text.trim_end()));
        }
    }
    if err.message().contains("missing field") {
        out.push("  （字段缺失：缺哪个 section/字段就补哪一行；字段名拼错也会报 missing field）".to_string());
    }
    out.push("  编辑配置: friend --edit-config".to_string());
    out
}

/// 启动前的配置校验：返回所有问题（空表示可以聊）
fn config_problems(config: &Config) -> Vec<String> {
    let mut problems = Vec::new();

    if config.api.key.trim().is_empty() {
        problems.push(
            "[api] key 为空：填入 API 密钥，否则首个请求只会拿到 401".to_string(),
        );
    }

    let endpoint = config.api.endpoint.trim();
    if endpoint.is_empty() {
        problems.push(
            "[api] endpoint 为空：填完整地址，如 https://api.deepseek.com/v1/chat/completions"
                .to_string(),
        );
    } else if !(endpoint.starts_with("http://") || endpoint.starts_with("https://")) {
        problems.push(format!(
            "[api] endpoint 必须以 http:// 或 https:// 开头，当前是「{}」",
            endpoint
        ));
    }

    if config.api.model.trim().is_empty() {
        problems.push("[api] model 为空：填模型名，如 deepseek-chat".to_string());
    }

    let temperature = config.personality.temperature;
    if !(0.0..=2.0).contains(&temperature) {
        problems.push(format!(
            "[personality] temperature 应在 0.0-2.0 之间，当前是 {}",
            temperature
        ));
    }

    if config.chat.max_context == 0 {
        problems.push("[chat] max_context 至少为 1".to_string());
    }

    problems
}

/// 有问题就在发出第一条消息之前拦住（而不是等 401 / 超时）
fn ensure_config_ready(config: &Config, path: &Path) {
    let problems = config_problems(config);
    if problems.is_empty() {
        return;
    }
    eprintln!("{} 配置不完整，先改配置再聊：", ansi("error", "错误:"));
    for problem in &problems {
        eprintln!("  - {}", problem);
    }
    eprintln!("  配置文件: {}", path.display());
    eprintln!("  编辑配置: friend --edit-config");
    process::exit(1);
}

// ======================== 对话管理 ========================

fn conv_filename(id: &str) -> String {
    format!("{}.json", id)
}

/// 会话 id：创建时刻 + 进程号。同一会话内固定不变，跨会话不会撞名
fn new_conversation_id() -> String {
    format!("{}-{}", chrono_now(), process::id())
}

fn new_conversation(config: &Config, messages: &[Message]) -> Conversation {
    Conversation {
        id: new_conversation_id(),
        personality: config.personality.name.clone(),
        created_at: now_timestamp(),
        updated_at: now_timestamp(),
        messages: messages.to_vec(),
    }
}

/// 旧格式文件没有 id：用文件名（去掉 .json）补齐，续聊时即可写回同一个文件
fn ensure_id(conv: &mut Conversation, filename: &str) {
    if conv.id.is_empty() {
        conv.id = filename.trim_end_matches(".json").to_string();
    }
}

/// 始终写会话自己的那个文件（id 固定），不再按秒级时间戳把一次聊天拆成多个文件
fn save_conversation(conv: &mut Conversation) -> String {
    if conv.id.is_empty() {
        conv.id = new_conversation_id();
    }
    let dir = conv_dir();
    let filename = conv_filename(&conv.id);
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
    let mut conv: Conversation = serde_json::from_str(&content).ok()?;
    ensure_id(&mut conv, filename);
    Some(conv)
}

/// 同一个 id 只保留 updated_at 最新的一条（旧版本可能把一次聊天写成多个文件）
fn dedup_by_id(mut list: Vec<(String, Conversation)>) -> Vec<(String, Conversation)> {
    list.sort_by(|a, b| b.1.updated_at.cmp(&a.1.updated_at));
    let mut seen = HashSet::new();
    list.retain(|(_, conv)| seen.insert(conv.id.clone()));
    list
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

    // 同一 id 只留最新一条，整体按更新时间倒序（最新的在前）
    dedup_by_id(list)
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

/// 把人格设定与最近的训练经验拼成 system prompt：新会话与 --continue 共用
fn build_system_prompt(config: &Config) -> String {
    compose_system_prompt(&config.personality.system_prompt, &load_training_examples())
}

/// 最近 5 条训练经验以 [经验] 形式追加在人格设定之后
fn compose_system_prompt(base: &str, examples: &[TrainingExample]) -> String {
    let mut prompt = base.to_string();
    let training_context: String = examples
        .iter()
        .rev()
        .take(5)
        .map(|e| format!("[经验] {} → 反馈: {}", e.context, e.user_feedback))
        .collect::<Vec<_>>()
        .join("\n");
    if !training_context.is_empty() {
        prompt.push_str(&format!(
            "\n\n以下是你从之前对话中学到的经验，请谨记并改进：\n{}",
            training_context
        ));
    }
    prompt
}

// ======================== 中断处理（Ctrl+C） ========================

/// 只在「等待流式响应」期间接管 Ctrl+C：置位后由流式循环收尾保存；
/// 其余时刻不接管，Ctrl+C 保持系统默认（直接退出）。
#[cfg(windows)]
mod interrupt {
    use std::sync::atomic::{AtomicBool, Ordering};

    static INTERRUPTED: AtomicBool = AtomicBool::new(false);

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetConsoleCtrlHandler(
            handler: Option<unsafe extern "system" fn(u32) -> i32>,
            add: i32,
        ) -> i32;
    }

    unsafe extern "system" fn handler(_ctrl_type: u32) -> i32 {
        // 返回 1 = 已处理，进程不退出；由流式循环看到标志后收尾
        INTERRUPTED.store(true, Ordering::SeqCst);
        1
    }

    pub fn install() {
        unsafe {
            // 以 CREATE_NEW_PROCESS_GROUP 拉起的进程（start / 计划任务 / 脚本）默认忽略 Ctrl+C，
            // 先恢复常规处理，否则自定义 handler 根本收不到事件
            SetConsoleCtrlHandler(None, 0);
            SetConsoleCtrlHandler(Some(handler), 1);
        }
    }

    pub fn uninstall() {
        unsafe {
            SetConsoleCtrlHandler(Some(handler), 0);
        }
    }

    /// 读取并清空标志
    pub fn take() -> bool {
        INTERRUPTED.swap(false, Ordering::SeqCst)
    }
}

#[cfg(not(windows))]
mod interrupt {
    pub fn install() {}
    pub fn uninstall() {}
    pub fn take() -> bool {
        false
    }
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
        // reasoning_content 用暗灰，和正文区分开
        "thinking" => format!("\x1b[90m{}\x1b[0m", text),
        "violet" => format!("\x1b[38;5;219m{}\x1b[0m", text),
        "violet_dim" => format!("\x1b[38;5;141m{}\x1b[0m", text),
        _ => text.to_string(),
    }
}

/// 按字符（不是字节）截断预览文本，避免把中文/emoji 切成半个字符导致 panic
fn preview(s: &str, n_chars: usize) -> String {
    let mut out: String = s.chars().take(n_chars).collect();
    if s.chars().count() > n_chars {
        out.push_str("...");
    }
    out
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
    println!(
        "  {:<20} {}",
        ansi("highlight", "Ctrl+C"),
        "AI 回复中按下：中断本次回复，已收到的内容会保存"
    );
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

/// 单次回复的结果：正文 + 推理内容（reasoning_content）+ 是否被 Ctrl+C 中断
#[derive(Debug, Default, PartialEq)]
struct StreamOutcome {
    content: String,
    reasoning: String,
    interrupted: bool,
}

fn chat_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(CONNECT_TIMEOUT_SECS))
        .timeout_read(Duration::from_secs(READ_TIMEOUT_SECS))
        .build()
}

/// ureq / 系统错误里的超时判定（英文/中文/常见 socket 错误码都判）
fn looks_like_timeout(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("timed out")
        || lower.contains("timeout")
        || msg.contains("超时")
        || msg.contains("10060")
        || msg.contains("没有正确答复")
        || msg.contains("没有响应")
}

/// 去掉 ureq 层层包裹的 `Network Error: ` 前缀，只留真正的原因
fn clean_transport_error(raw: &str) -> String {
    let mut rest = raw.trim();
    while let Some(stripped) = rest.strip_prefix("Network Error: ") {
        rest = stripped.trim_start();
    }
    rest.to_string()
}

/// 从错误文本里抠出 `os error N` 的码值，用于给出简短提示
fn os_error_code(msg: &str) -> Option<String> {
    let idx = msg.find("os error ")?;
    let digits: String = msg[idx + "os error ".len()..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        None
    } else {
        Some(digits)
    }
}

/// 解析一行 SSE：空行、`:` 开头的注释、`event:`/`id:`/`retry:` 等非 data 字段都返回 None；
/// `data:` 后面的空格可有可无（兼容 `data:{...}` 与 `data: {...}`）
fn sse_data(line: &str) -> Option<&str> {
    let trimmed = line.trim_end_matches(['\r', '\n']).trim_start();
    if trimmed.is_empty() || trimmed.starts_with(':') {
        return None;
    }
    let rest = trimmed.strip_prefix("data:")?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

fn read_error(e: io::Error) -> String {
    let msg = e.to_string();
    if e.kind() == io::ErrorKind::TimedOut
        || e.kind() == io::ErrorKind::WouldBlock
        || looks_like_timeout(&msg)
    {
        format!("读取响应超时（{} 秒没有新数据）：{}", READ_TIMEOUT_SECS, msg)
    } else {
        format!("读取响应失败: {}", msg)
    }
}

/// 跳过空行与注释，拿到第一行有内容的数据
fn read_first_payload<R: BufRead>(reader: &mut R) -> Result<String, String> {
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).map_err(read_error)? == 0 {
            return Err("服务端没有返回任何内容".to_string());
        }
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with(':') {
            continue;
        }
        return Ok(line.clone());
    }
}

/// 非流式（一次性 JSON）响应
fn read_plain_json<R: BufRead>(reader: &mut R, first: String) -> Result<StreamOutcome, String> {
    let mut json_str = first;
    let mut line = String::new();
    while reader.read_line(&mut line).map_err(read_error)? > 0 {
        json_str.push_str(&line);
        line.clear();
    }
    let resp: serde_json::Value =
        serde_json::from_str(&json_str).map_err(|e| format!("解析响应失败: {}", e))?;
    let content = resp
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();
    println!("{}", content);
    Ok(StreamOutcome {
        content,
        ..Default::default()
    })
}

/// 逐行消费 SSE：
/// - 跳过空行、`:` 注释、`event:` 等非 data 字段（不再「首行不是 data: 就报未知格式」）
/// - `data:` 后空格可选，`data: [DONE]` 与 `finish_reason` 都作为结束信号
/// - 坏 JSON 单块跳过，不中断整段回复
/// - 每次读下一行前调用 `stop_requested()`，为 true 就中断并保留已收到的内容
fn stream_sse<R, S, C, T>(
    reader: &mut R,
    first_line: &str,
    mut stop_requested: S,
    mut emit_content: C,
    mut emit_reasoning: T,
) -> Result<StreamOutcome, String>
where
    R: BufRead,
    S: FnMut() -> bool,
    C: FnMut(&str),
    T: FnMut(&str),
{
    let mut outcome = StreamOutcome::default();
    let mut line = first_line.to_string();
    let mut finished = false;

    loop {
        if stop_requested() {
            outcome.interrupted = true;
            break;
        }

        if let Some(data) = sse_data(&line) {
            let data = data.trim();
            if data == "[DONE]" {
                finished = true;
            } else if !data.is_empty() {
                if let Ok(chunk) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(choice) = chunk.get("choices").and_then(|c| c.get(0)) {
                        if let Some(reasoning) = choice
                            .get("delta")
                            .and_then(|d| d.get("reasoning_content"))
                            .and_then(|r| r.as_str())
                        {
                            if !reasoning.is_empty() {
                                outcome.reasoning.push_str(reasoning);
                                emit_reasoning(reasoning);
                            }
                        }
                        if let Some(content) = choice
                            .get("delta")
                            .and_then(|d| d.get("content"))
                            .and_then(|c| c.as_str())
                        {
                            if !content.is_empty() {
                                outcome.content.push_str(content);
                                emit_content(content);
                            }
                        }
                        if let Some(finish) = choice.get("finish_reason").and_then(|f| f.as_str()) {
                            if !finish.is_empty() {
                                finished = true;
                            }
                        }
                    }
                }
            }
        }

        if finished {
            break;
        }

        line.clear();
        if reader.read_line(&mut line).map_err(read_error)? == 0 {
            break;
        }
    }

    Ok(outcome)
}

fn send_chat(config: &Config, messages: &[Message]) -> Result<StreamOutcome, String> {
    let url = config.api.endpoint.trim_end_matches('/').to_string();
    let body = build_request_body(config, messages);
    let agent = chat_agent();

    let response = match agent
        .post(&url)
        .set("Authorization", &format!("Bearer {}", config.api.key))
        .set("Accept", "text/event-stream")
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
        Err(e) => {
            let detail = clean_transport_error(&e.to_string());
            if looks_like_timeout(&detail) {
                let code = os_error_code(&detail)
                    .map(|c| format!("，os error {}", c))
                    .unwrap_or_default();
                return Err(format!(
                    "请求超时（连接 {} 秒 / 读取 {} 秒）：服务器没有及时响应，检查 endpoint 或网络{}",
                    CONNECT_TIMEOUT_SECS, READ_TIMEOUT_SECS, code
                ));
            }
            return Err(format!("API 请求失败: {}", detail));
        }
    };

    let mut reader = BufReader::new(response.into_reader());
    let first_line = read_first_payload(&mut reader)?;

    // 一次性 JSON（部分代理不认 stream=true）
    if first_line.starts_with('{') {
        return read_plain_json(&mut reader, first_line);
    }

    // 不是 SSE 也不是 JSON：报错时只截一小段，避免把整页 HTML 刷到屏幕上
    if sse_data(&first_line).is_none() {
        return Err(format!("未知的响应格式: {}", preview(first_line.trim(), 80)));
    }

    interrupt::install();
    let outcome = stream_sse(
        &mut reader,
        &first_line,
        interrupt::take,
        |content| {
            print!("{}", content);
            io::stdout().flush().ok();
        },
        |reasoning| {
            print!("{}", ansi("thinking", reasoning));
            io::stdout().flush().ok();
        },
    );
    interrupt::uninstall();

    println!();
    outcome
}

// ======================== 聊天循环 ========================

fn run_chat(config: &Config, messages: &mut Vec<Message>, mut conv: Conversation) {
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

    // conv 由调用方创建（新会话）或复用（--continue）；id 固定，自动保存始终覆盖同一个文件
    conv.messages = messages.clone();

    loop {
        // 读取用户输入
        print!("{} ", ansi("user", "你:"));
        io::stdout().flush().ok();

        let mut input = String::new();
        let read = io::stdin().read_line(&mut input).unwrap_or_else(|e| {
            eprintln!("{} 读取输入失败: {}", ansi("error", "错误"), e);
            0
        });

        // stdin 结束（Ctrl+Z / 管道读完）：不要空转，直接退出
        if read == 0 {
            println!("{} 输入结束，退出对话", ansi("system", "→"));
            break;
        }

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
            Ok(outcome) => {
                if outcome.content.is_empty() {
                    // 移除无效的用户消息
                    messages.pop();
                    if outcome.interrupted {
                        println!("{} 已中断，还没收到内容", ansi("system", "→"));
                    } else {
                        println!("{} AI 返回了空响应，请重试", ansi("error", "⚠"));
                    }
                    continue;
                }
                if outcome.interrupted {
                    println!(
                        "{} 已中断，保留已收到的 {} 个字符",
                        ansi("system", "→"),
                        outcome.content.chars().count()
                    );
                }
                messages.push(Message {
                    role: "assistant".to_string(),
                    content: outcome.content.clone(),
                });
                conv.updated_at = now_timestamp();
                conv.messages = messages.clone();

                // 自动保存（始终写同一个文件）；被中断的半截回复也落盘，否则 --continue 看不到
                if config.chat.auto_save || outcome.interrupted {
                    save_conversation(&mut conv);
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
///
/// `messages` 用 `&mut Vec<Message>`：为后续 `/undo`（pop 最后一条）留出入口
fn handle_command(
    input: &str,
    config: &Config,
    conv: &mut Conversation,
    messages: &mut Vec<Message>,
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
            conv.messages = messages.clone();
            conv.updated_at = now_timestamp();
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
                    println!("     {:>6}{}", "", preview(&first_user_msg.content, 30));
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
            ensure_config_ready(&config, &config_file_for(persona_ref));
            // 训练经验在这里也要注入，否则续聊时 /train 记录等于白记
            let system_prompt = build_system_prompt(&config);
            match get_latest_conversation() {
                Some((_filename, conv)) => {
                    let mut messages = conv.messages.clone();
                    if messages.is_empty() || messages[0].role != "system" {
                        messages.insert(
                            0,
                            Message {
                                role: "system".to_string(),
                                content: system_prompt,
                            },
                        );
                    } else {
                        messages[0].content = system_prompt;
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
                        println!(
                            "{}: {}",
                            ansi(if msg.role == "user" { "user" } else { col }, tag),
                            preview(&msg.content, 60)
                        );
                    }
                    if messages.iter().filter(|m| m.role != "system").count() > 4 {
                        println!("{} ... 还有更多历史消息", ansi("system", ""));
                    }
                    println!();
                    // 复用原会话：id 相同 → 自动保存写回同一个文件
                    run_chat(&config, &mut messages, conv);
                }
                None => {
                    println!("暂无历史对话，开始新对话。");
                    let mut messages = vec![Message {
                        role: "system".to_string(),
                        content: system_prompt,
                    }];
                    let conv = new_conversation(&config, &messages);
                    run_chat(&config, &mut messages, conv);
                }
            }
        }
        "" => {
            let config = load_config(persona_ref);
            ensure_config_ready(&config, &config_file_for(persona_ref));
            let mut messages = vec![Message {
                role: "system".to_string(),
                content: build_system_prompt(&config),
            }];
            let conv = new_conversation(&config, &messages);
            run_chat(&config, &mut messages, conv);
        }
        _ => {
            eprintln!("错误: 未知参数 '{}'", cmd);
            println!();
            print_usage();
            process::exit(1);
        }
    }
}

// ======================== 单元测试 ========================

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: &str, content: &str) -> Message {
        Message {
            role: role.to_string(),
            content: content.to_string(),
        }
    }

    fn conv(id: &str, updated_at: &str) -> Conversation {
        Conversation {
            id: id.to_string(),
            personality: "测试".to_string(),
            created_at: "2026-11-21 09:00".to_string(),
            updated_at: updated_at.to_string(),
            messages: vec![msg("user", "你好")],
        }
    }

    #[test]
    fn preview_pure_chinese_truncates_by_chars() {
        let s = "你好世界".repeat(10); // 40 个汉字
        let p = preview(&s, 30);
        assert!(p.ends_with("..."));
        assert_eq!(p, format!("{}...", "你好世界".repeat(7) + "你好"));
    }

    #[test]
    fn preview_pure_ascii_truncates_by_chars() {
        let s = "a".repeat(100);
        assert_eq!(preview(&s, 60), format!("{}...", "a".repeat(60)));
    }

    #[test]
    fn preview_mixed_text_never_panics() {
        // 旧实现用 &content[..30] / [..60] 按字节切，中英混排时会切在字符中间 panic
        let s = "你好hello世界world你好hello世界world你好hello世界world";
        for n in [0usize, 1, 3, 7, 30, 60] {
            let p = preview(s, n);
            assert!(p.chars().count() <= n + 3);
        }
        assert!(preview(s, 30).ends_with("..."));
        assert_eq!(preview(s, 30).chars().count(), 33);
    }

    #[test]
    fn preview_keeps_text_at_or_below_limit() {
        assert_eq!(preview("短文本", 30), "短文本");
        assert_eq!(preview(&"中".repeat(30), 30), "中".repeat(30));
        assert_eq!(preview("", 30), "");
    }

    #[test]
    fn new_conversation_id_is_filename_safe() {
        let id = new_conversation_id();
        assert!(!id.is_empty());
        assert!(id
            .chars()
            .all(|c| c.is_ascii_digit() || c == '-' || c == '_'));
        assert_eq!(conv_filename(&id), format!("{}.json", id));
    }

    #[test]
    fn legacy_conversation_gets_id_from_filename() {
        // v1.0.0 写出的文件没有 id 字段
        let json = r#"{"personality":"朋友","created_at":"2026-11-21 09:00","updated_at":"2026-11-21 09:10","messages":[{"role":"user","content":"你好"}]}"#;
        let mut c: Conversation = serde_json::from_str(json).unwrap();
        assert!(c.id.is_empty(), "旧格式应解析出空 id");
        ensure_id(&mut c, "2026-11-21_09-00-00.json");
        assert_eq!(c.id, "2026-11-21_09-00-00");
        assert_eq!(conv_filename(&c.id), "2026-11-21_09-00-00.json");
    }

    #[test]
    fn dedup_by_id_keeps_newest() {
        let list = vec![
            ("a.json".to_string(), conv("x", "2026-11-21 09:00")),
            ("b.json".to_string(), conv("x", "2026-11-21 10:00")),
            ("c.json".to_string(), conv("y", "2026-11-21 08:00")),
        ];
        let out = dedup_by_id(list);
        assert_eq!(out.len(), 2, "同一 id 只保留一条");
        assert_eq!(out[0].1.id, "x");
        assert_eq!(out[0].1.updated_at, "2026-11-21 10:00", "保留 update 最新的那条");
        assert_eq!(out[1].1.id, "y");
    }

    #[test]
    fn system_prompt_includes_recent_training_examples() {
        let examples: Vec<TrainingExample> = (1..=7)
            .map(|i| TrainingExample {
                context: format!("上下文{}", i),
                user_feedback: format!("反馈{}", i),
                learned_at: format!("时间{}", i),
            })
            .collect();
        let prompt = compose_system_prompt("人格设定", &examples);
        assert!(prompt.starts_with("人格设定"));
        assert!(prompt.contains("[经验] 上下文7 → 反馈: 反馈7"));
        assert!(prompt.contains("[经验] 上下文3 → 反馈: 反馈3"));
        assert!(!prompt.contains("上下文2"), "只保留最近 5 条");
        assert!(
            prompt.find("上下文7").unwrap() < prompt.find("上下文3").unwrap(),
            "最新的经验排在前面"
        );
    }

    #[test]
    fn system_prompt_without_training_equals_base() {
        assert_eq!(compose_system_prompt("人格", &[]), "人格");
    }

    // ---------- 配置缺省值 ----------

    const MINIMAL_CONFIG: &str = r#"
[api]
endpoint = "https://example.com/v1/chat/completions"
key = "sk-test"
model = "test-model"
"#;

    #[test]
    fn config_without_personality_and_chat_tables_uses_defaults() {
        // 验收项：删掉整段 [chat]、[personality] 仍能启动并落默认值
        let cfg: Config = toml::from_str(MINIMAL_CONFIG).expect("缺 [chat]/[personality] 也要能解析");
        assert!(cfg.chat.auto_save, "auto_save 缺省 true");
        assert_eq!(cfg.chat.max_context, DEFAULT_MAX_CONTEXT);
        assert_eq!(cfg.personality.name, "朋友");
        assert_eq!(cfg.personality.temperature, DEFAULT_TEMPERATURE);
        assert_eq!(cfg.personality.system_prompt, default_system_prompt());
        assert!(cfg.personality.greeting.is_empty(), "问候语缺省为空");
    }

    #[test]
    fn config_partial_chat_table_fills_only_missing_fields() {
        let toml_str = r#"
[api]
endpoint = "http://127.0.0.1:1/v1"
key = "k"

[chat]
max_context = 5
"#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert!(cfg.chat.auto_save, "缺 auto_save 时用 true");
        assert_eq!(cfg.chat.max_context, 5, "写了的字段不被覆盖");
        assert_eq!(cfg.api.model, DEFAULT_MODEL, "model 缺省");
        assert_eq!(cfg.personality.temperature, DEFAULT_TEMPERATURE);
    }

    #[test]
    fn config_missing_key_parses_then_fails_startup_check() {
        // key 现在可以缺省（解析不报错），但启动校验必须拦住
        let toml_str = "[api]\nendpoint = \"https://example.com/v1\"\nmodel = \"m\"\n";
        let cfg: Config = toml::from_str(toml_str).unwrap();
        let problems = config_problems(&cfg);
        assert!(
            problems.iter().any(|p| p.contains("key 为空")),
            "应提示 key 为空: {:?}",
            problems
        );
    }

    #[test]
    fn config_problems_check_endpoint_temperature_and_context() {
        let mut cfg = default_config();
        cfg.api.key = "sk-x".to_string();
        assert!(config_problems(&cfg).is_empty(), "默认配置 + 有 key 应无问题");

        cfg.api.endpoint = "api.example.com/v1".to_string();
        assert!(config_problems(&cfg)
            .iter()
            .any(|p| p.contains("http://") && p.contains("https://")));

        cfg.api.endpoint = "https://api.example.com/v1".to_string();
        cfg.personality.temperature = 3.0;
        assert!(config_problems(&cfg)
            .iter()
            .any(|p| p.contains("temperature")));

        cfg.personality.temperature = 0.8;
        cfg.chat.max_context = 0;
        assert!(config_problems(&cfg)
            .iter()
            .any(|p| p.contains("max_context")));

        cfg.chat.max_context = 30;
        cfg.api.model = "  ".to_string();
        assert!(config_problems(&cfg)
            .iter()
            .any(|p| p.contains("model 为空")));
    }

    #[test]
    fn format_config_error_reports_line_and_missing_field() {
        // 缺 endpoint（必填字段）→ toml 报 missing field，报告里要带位置与字段提示
        let content = "[api]\nkey = \"x\"\n";
        let err = toml::from_str::<Config>(content).unwrap_err();
        let report = format_config_error(content, &err, Path::new("C:/tmp/config.toml")).join("\n");
        assert!(report.contains("missing field"), "{}", report);
        assert!(report.contains("第 1 行"), "{}", report);
        assert!(report.contains("C:/tmp/config.toml"), "{}", report);
        assert!(report.contains("缺哪个 section/字段"), "{}", report);
    }

    #[test]
    fn line_col_counts_from_one() {
        let content = "a\nbb\nccc";
        assert_eq!(line_col(content, 0), (1, 1));
        assert_eq!(line_col(content, 2), (2, 1));
        assert_eq!(line_col(content, 5), (3, 1));
        assert_eq!(line_col(content, 7), (3, 3));
        assert_eq!(line_col(content, 999), (3, 4), "越界按末尾算");
    }

    /// 只在临时目录确实可写时才跑文件测试（受限沙箱里 %TEMP% 可能不可写）
    fn scratch_dir(tag: &str) -> Option<PathBuf> {
        let dir = std::env::temp_dir().join(format!("friend-test-{}-{}", tag, process::id()));
        fs::create_dir_all(&dir).ok()?;
        fs::write(dir.join(".probe"), b"ok").ok()?;
        Some(dir)
    }

    #[test]
    fn write_atomic_replaces_target_and_leaves_no_tmp() {
        let dir = match scratch_dir("atomic") {
            Some(d) => d,
            None => {
                eprintln!("跳过：临时目录不可写");
                return;
            }
        };
        let path = dir.join("config.toml");
        fs::write(&path, "old-content").unwrap();
        write_atomic(&path, "new-content").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new-content");
        assert!(!tmp_path(&path).exists(), "临时文件应已改名消失");
        let _ = fs::remove_dir_all(&dir);
    }

    // ---------- SSE 容错 ----------

    #[test]
    fn sse_data_accepts_optional_space_and_ignores_other_lines() {
        assert_eq!(sse_data("data: {\"a\":1}\r\n"), Some("{\"a\":1}"));
        assert_eq!(sse_data("data:{\"a\":1}\n"), Some("{\"a\":1}"), "无空格也要认");
        assert_eq!(sse_data("data: [DONE]"), Some("[DONE]"));
        assert_eq!(sse_data("data:"), Some(""));
        assert_eq!(sse_data(": keep-alive"), None, "注释行跳过");
        assert_eq!(sse_data("event: message"), None, "event 字段跳过");
        assert_eq!(sse_data("id: 42"), None);
        assert_eq!(sse_data(""), None);
        assert_eq!(sse_data("   "), None);
    }

    /// 拆成「首行 + 其余」喂给 stream_sse（真实调用就是这么进来的）
    fn run_stream<F: FnMut() -> bool>(payload: &str, stop: F) -> StreamOutcome {
        let mut lines = payload.lines();
        let first = format!("{}\n", lines.next().unwrap_or(""));
        let rest: String = lines.map(|l| format!("{}\n", l)).collect();
        let mut reader = BufReader::new(rest.as_bytes());
        stream_sse(&mut reader, &first, stop, |_| {}, |_| {}).unwrap()
    }

    #[test]
    fn stream_sse_normal_path_collects_content_reasoning_and_stops_at_done() {
        let payload = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"你\"}}]}\n",
            "\n",
            ": keep-alive\n",
            "event: message\n",
            "data:{\"choices\":[{\"delta\":{\"content\":\"好\"}}]}\n",
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"先想想\"}}]}\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n",
            "data: [DONE]\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"[DONE] 之后不该被读到\"}}]}\n",
        );
        let outcome = run_stream(payload, || false);
        assert_eq!(outcome.content, "你好", "正常流式内容不能因为容错而丢");
        assert_eq!(outcome.reasoning, "先想想");
        assert!(!outcome.interrupted);
    }

    #[test]
    fn stream_sse_reports_interrupt_and_keeps_partial_text() {
        // 第一次检查不中断、第二次也不中断，第三次中断 → 只保留前两块
        let payload = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"开头\"}}]}\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"前半\"}}]}\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"后半\"}}]}\n",
        );
        let mut checks = 0;
        let outcome = run_stream(payload, move || {
            checks += 1;
            checks > 2
        });
        assert!(outcome.interrupted, "中断标志要带出来");
        assert_eq!(outcome.content, "开头前半");
        assert!(!outcome.content.contains("后半"));
    }

    #[test]
    fn stream_sse_skips_malformed_json_without_failing() {
        let payload = concat!(
            "data: not-json\n",
            "data: {\"choices\":[]}\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n",
            "data: [DONE]\n",
        );
        let outcome = run_stream(payload, || false);
        assert_eq!(outcome.content, "ok");
        assert!(!outcome.interrupted);
    }

    #[test]
    fn stream_sse_ending_without_done_returns_what_it_got() {
        let payload = "data: {\"choices\":[{\"delta\":{\"content\":\"半截\"}}]}\n";
        let outcome = run_stream(payload, || false);
        assert_eq!(outcome.content, "半截");
        assert!(!outcome.interrupted, "连接提前结束不等于被中断");
    }

    #[test]
    fn transport_errors_are_cleaned_and_recognized_as_timeout() {
        // ureq 会把 Network Error 前缀叠两层，界面上只留一层原因
        let raw = "Network Error: Network Error: Error encountered in the status line: 由于连接方在一段时间后没有正确答复 (os error 10060)";
        let cleaned = clean_transport_error(raw);
        assert!(!cleaned.starts_with("Network Error"), "{}", cleaned);
        assert!(cleaned.contains("10060"), "{}", cleaned);
        assert!(looks_like_timeout(&cleaned), "10060 要判定为超时");
        assert!(looks_like_timeout("operation timed out"));
        assert!(looks_like_timeout("连接超时"));
        assert!(!looks_like_timeout("400 Bad Request"));
        assert_eq!(os_error_code(&cleaned).as_deref(), Some("10060"));
        assert_eq!(os_error_code("no code here"), None);
    }
}
