use crate::config::{AppConfig, DesktopGroup, DesktopItem, DesktopItemKind, GroupLayout};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub fn scan_into_config(config: &mut AppConfig) -> Result<usize, String> {
    let mut scanned = scan_desktop_items(config)?;
    if scanned.is_empty() {
        return Ok(0);
    }

    let group_index = ensure_unclassified_group(config);
    let mut existing: HashSet<String> = config
        .groups
        .iter()
        .flat_map(|group| group.items.iter().map(|item| item.path.to_lowercase()))
        .collect();

    let mut added = 0usize;
    for item in scanned.drain(..) {
        if existing.insert(item.path.to_lowercase()) {
            config.groups[group_index].items.push(item);
            place_item_in_group(&mut config.groups[group_index]);
            added += 1;
        }
    }
    Ok(added)
}

/// 当前配置下需要监视/扫描的桌面根目录
pub fn scan_roots(config: &AppConfig) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if config.scan.include_user_desktop {
        if let Some(path) = dirs::desktop_dir() {
            roots.push(path);
        }
    }
    if config.scan.include_public_desktop {
        roots.push(PathBuf::from("C:\\Users\\Public\\Desktop"));
    }
    roots
}

/// 路径是否在忽略清单里（大小写不敏感）
pub fn is_ignored(config: &AppConfig, path: &Path) -> bool {
    let key = path.to_string_lossy().to_lowercase();
    config.scan.ignore.iter().any(|ignored| ignored == &key)
}

/// 把路径加入忽略清单；返回是否新增
pub fn add_ignored(config: &mut AppConfig, path: &Path) -> bool {
    let key = path.to_string_lossy().trim().to_lowercase();
    if key.is_empty() || config.scan.ignore.iter().any(|ignored| ignored == &key) {
        return false;
    }
    config.scan.ignore.push(key);
    true
}

/// 从忽略清单移除路径（用户主动把文件拖回面板时撤销忽略）
pub fn remove_ignored(config: &mut AppConfig, path: &Path) -> bool {
    let key = path.to_string_lossy().trim().to_lowercase();
    let before = config.scan.ignore.len();
    config.scan.ignore.retain(|ignored| ignored != &key);
    before != config.scan.ignore.len()
}

pub fn scan_desktop_items(config: &AppConfig) -> Result<Vec<DesktopItem>, String> {
    let mut items = Vec::new();
    for root in scan_roots(config) {
        if !root.exists() {
            continue;
        }
        let entries = fs::read_dir(&root)
            .map_err(|e| format!("读取桌面目录 {} 失败: {}", root.display(), e))?;
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let path = entry.path();
            if is_hidden_system_name(&path) || is_ignored(config, &path) {
                continue;
            }
            if let Some(item) = item_from_path(&path) {
                items.push(item);
            }
        }
    }
    items.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(items)
}

/// 读取桌面根目录当前的全部条目（不含桌面系统文件），用于实时同步比较
pub fn snapshot_paths(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for root in roots {
        let Ok(entries) = fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if is_hidden_system_name(&path) {
                continue;
            }
            paths.push(path);
        }
    }
    paths.sort_by_key(|path| path.to_string_lossy().to_lowercase());
    paths
}

/// 实时同步一次快照：只把新出现的文件补进「未分类」，不自动删除已分类项目
/// （文件消失由 UI 侧按路径存在性标记 missing，交给用户决定）。
pub struct SnapshotOutcome {
    pub added: usize,
}

pub fn apply_snapshot(config: &mut AppConfig, snapshot: &[PathBuf]) -> SnapshotOutcome {
    let mut existing: HashSet<String> = config
        .groups
        .iter()
        .flat_map(|group| group.items.iter().map(|item| item.path.to_lowercase()))
        .collect();
    let mut new_items = Vec::new();
    for path in snapshot {
        if is_hidden_system_name(path) || is_ignored(config, path) {
            continue;
        }
        let key = path.to_string_lossy().to_lowercase();
        if existing.contains(&key) {
            continue;
        }
        if !path.exists() {
            continue;
        }
        if let Some(item) = item_from_path(path) {
            existing.insert(key);
            new_items.push(item);
        }
    }
    if new_items.is_empty() {
        return SnapshotOutcome { added: 0 };
    }
    let group_index = ensure_unclassified_group(config);
    let mut added = 0usize;
    for item in new_items {
        config.groups[group_index].items.push(item);
        place_item_in_group(&mut config.groups[group_index]);
        added += 1;
    }
    SnapshotOutcome { added }
}

pub fn open_item(item: &DesktopItem) -> Result<(), String> {
    match item.kind {
        DesktopItemKind::Command => Command::new(&item.path)
            .args(&item.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("{} ({})", item.path, e)),
        DesktopItemKind::Folder
        | DesktopItemKind::Shortcut
        | DesktopItemKind::Url
        | DesktopItemKind::File => open_shell_path(&item.path),
    }
}

pub fn item_badge(kind: &DesktopItemKind) -> (&'static str, (u8, u8, u8)) {
    match kind {
        DesktopItemKind::Shortcut => ("S", (77, 134, 255)),
        DesktopItemKind::Url => ("W", (43, 177, 152)),
        DesktopItemKind::Folder => ("F", (245, 170, 64)),
        DesktopItemKind::File => ("D", (124, 136, 151)),
        DesktopItemKind::Command => ("C", (168, 112, 255)),
    }
}

fn ensure_unclassified_group(config: &mut AppConfig) -> usize {
    if let Some(index) = config
        .groups
        .iter()
        .position(|group| group.id == "unclassified")
    {
        return index;
    }
    config.groups.insert(
        0,
        crate::config::DesktopGroup {
            id: "unclassified".to_string(),
            name: "未分类".to_string(),
            x: 28.0,
            y: 34.0,
            width: 300.0,
            height: 220.0,
            collapsed: false,
            position_locked: false,
            size_locked: false,
            layout: crate::config::GroupLayout::Grid,
            icon_size: 48.0,
            items: Vec::new(),
        },
    );
    0
}

#[cfg(windows)]
fn open_shell_path(path: &str) -> Result<(), String> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let path_wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let verb_wide: Vec<u16> = "open".encode_utf16().chain(std::iter::once(0)).collect();
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb_wide.as_ptr(),
            path_wide.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if (result as isize) <= 32 {
        Err(format!("无法打开 {}", path))
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn open_shell_path(_path: &str) -> Result<(), String> {
    Err("桌面项目打开功能当前仅支持 Windows".to_string())
}

pub(crate) fn item_from_path(path: &Path) -> Option<DesktopItem> {
    let name = path.file_stem()?.to_string_lossy().to_string();
    let kind = if path.is_dir() {
        DesktopItemKind::Folder
    } else {
        match path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_lowercase()
            .as_str()
        {
            "lnk" => DesktopItemKind::Shortcut,
            "url" => DesktopItemKind::Url,
            _ => DesktopItemKind::File,
        }
    };
    Some(DesktopItem {
        id: stable_id(path),
        name,
        path: path.display().to_string(),
        kind,
        args: Vec::new(),
        pinned: false,
        x: 0.0,
        y: 0.0,
    })
}

fn stable_id(path: &Path) -> String {
    path.display()
        .to_string()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

fn is_hidden_system_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.eq_ignore_ascii_case("desktop.ini"))
        .unwrap_or(false)
}

pub fn place_item_in_group(group: &mut DesktopGroup) {
    if group.items.is_empty() {
        return;
    }

    let index = group.items.len() - 1;
    let step_x = group.icon_size + 12.0;
    let step_y = group.icon_size + 30.0;
    match group.layout {
        GroupLayout::Horizontal => {
            group.items[index].x = index as f32 * step_x;
            group.items[index].y = 0.0;
        }
        GroupLayout::Grid | GroupLayout::Free => {
            let columns = if group.width >= 420.0 {
                4
            } else if group.width >= 300.0 {
                3
            } else {
                2
            };
            group.items[index].x = (index % columns) as f32 * step_x;
            group.items[index].y = (index / columns) as f32 * step_y;
        }
    }
}

// ---------- 真实图标提取（Windows） ----------

#[cfg(windows)]
use windows_sys::Win32::UI::WindowsAndMessaging::HICON;

/// 提取文件/文件夹关联图标，返回 RGBA 像素（未预乘 alpha）。
/// 优先走系统图像列表（JUMBO 256px / EXTRALARGE 48px），失败再回退 32px 传统方式。
#[cfg(windows)]
pub fn extract_icon_rgba(path: &str) -> Option<(Vec<u8>, usize, usize)> {
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
    use windows_sys::Win32::UI::Shell::{
        SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_SHELLICONSIZE,
        SHGFI_SYSICONINDEX, SHIL_EXTRALARGE, SHIL_JUMBO,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::DestroyIcon;

    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();

    // 系统图像列表中的图标索引（会解析快捷方式的目标图标）
    let mut shfi: SHFILEINFOW = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        SHGetFileInfoW(
            wide.as_ptr(),
            FILE_ATTRIBUTE_NORMAL,
            &mut shfi,
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_SYSICONINDEX,
        )
    };

    if ok != 0 {
        // JUMBO：只接受真正的高清内容（内容包围盒 >= 96）。
        // 没有高清原图的图标会被系统在 256 画布里放大填充，需回退。
        let jumbo = unsafe { hicon_from_image_list(SHIL_JUMBO, shfi.iIcon) };
        if let Some(hicon) = jumbo {
            let out = unsafe { hicon_rgba_cropped(hicon, 96) };
            unsafe { DestroyIcon(hicon) };
            if out.is_some() {
                return out;
            }
        }
        // EXTRALARGE：48px 全幅图标
        let extra = unsafe { hicon_from_image_list(SHIL_EXTRALARGE, shfi.iIcon) };
        if let Some(hicon) = extra {
            let out = unsafe { hicon_rgba_cropped(hicon, 24) };
            unsafe { DestroyIcon(hicon) };
            if out.is_some() {
                return out;
            }
        }
    }

    // 回退：传统 32px 方式
    let mut shfi: SHFILEINFOW = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        SHGetFileInfoW(
            wide.as_ptr(),
            FILE_ATTRIBUTE_NORMAL,
            &mut shfi,
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON | SHGFI_SHELLICONSIZE,
        )
    };
    if ok == 0 || shfi.hIcon.is_null() {
        return None;
    }
    let out = unsafe { hicon_to_rgba(shfi.hIcon) };
    unsafe { DestroyIcon(shfi.hIcon) };
    out.and_then(|(buf, w, h)| Some(crop_to_content(&buf, w, h).unwrap_or((buf, w, h))))
}

/// 转换为 RGBA 并裁剪到内容包围盒；内容小于 min_content 视为伪高清，返回 None
unsafe fn hicon_rgba_cropped(hicon: HICON, min_content: usize) -> Option<(Vec<u8>, usize, usize)> {
    let (buf, w, h) = hicon_to_rgba(hicon)?;
    let cropped = crop_to_content(&buf, w, h).unwrap_or((buf, w, h));
    let (cw, ch) = (cropped.1, cropped.2);
    if cw < min_content || ch < min_content {
        return None;
    }
    Some(cropped)
}

/// 按 alpha 通道求内容包围盒并裁剪
fn crop_to_content(buf: &[u8], w: usize, h: usize) -> Option<(Vec<u8>, usize, usize)> {
    let mut min_x = w;
    let mut min_y = h;
    let mut max_x = 0usize;
    let mut max_y = 0usize;
    let mut any = false;
    for y in 0..h {
        for x in 0..w {
            if buf[(y * w + x) * 4 + 3] > 8 {
                any = true;
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }
    if !any {
        return None;
    }
    let cw = max_x - min_x + 1;
    let ch = max_y - min_y + 1;
    let mut out = Vec::with_capacity(cw * ch * 4);
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let i = (y * w + x) * 4;
            out.extend_from_slice(&buf[i..i + 4]);
        }
    }
    Some((out, cw, ch))
}

/// IImageList COM 接口（只声明用到的成员，其余用 usize 占位保持布局）
#[cfg(windows)]
#[repr(C)]
struct IImageListVtbl {
    query_interface: usize,
    add_ref: usize,
    release: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32,
    add: usize,
    replace_icon: usize,
    set_overlay_image: usize,
    replace: usize,
    add_masked: usize,
    draw: usize,
    remove: usize,
    get_icon: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        i32,
        u32,
        *mut windows_sys::Win32::UI::WindowsAndMessaging::HICON,
    ) -> windows_sys::core::HRESULT,
}

#[cfg(windows)]
#[repr(C)]
struct IImageList {
    vtbl: *const IImageListVtbl,
}

/// 从 Shell 系统图像列表取出指定索引的 HICON
#[cfg(windows)]
unsafe fn hicon_from_image_list(shil: u32, index: i32) -> Option<HICON> {
    use windows_sys::Win32::UI::Shell::SHGetImageList;
    use windows_sys::core::GUID;

    // IID_IImageList {46EB5926-582E-4017-9FDF-E8998DAA0950}
    const IID_IIMAGE_LIST: GUID = GUID {
        data1: 0x46EB5926,
        data2: 0x582E,
        data3: 0x4017,
        data4: [0x9F, 0xDF, 0xE8, 0x99, 0x8D, 0xAA, 0x09, 0x50],
    };
    const ILD_TRANSPARENT: u32 = 0x0000_0001;

    let mut list: *mut core::ffi::c_void = std::ptr::null_mut();
    if SHGetImageList(shil as i32, &IID_IIMAGE_LIST, &mut list) != 0 || list.is_null() {
        return None;
    }
    let vtbl = (*(list as *mut IImageList)).vtbl;
    let mut hicon: HICON = std::ptr::null_mut();
    let hr = ((*vtbl).get_icon)(list, index, ILD_TRANSPARENT, &mut hicon);
    ((*vtbl).release)(list);
    if hr == 0 && !hicon.is_null() {
        Some(hicon)
    } else {
        None
    }
}

/// HICON 转 RGBA 像素（ BGRA -> RGBA，处理 alpha 与预乘）
#[cfg(windows)]
unsafe fn hicon_to_rgba(hicon: HICON) -> Option<(Vec<u8>, usize, usize)> {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC,
        BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetIconInfo, ICONINFO};

    let mut info: ICONINFO = std::mem::zeroed();
    if GetIconInfo(hicon, &mut info) == 0 {
        return None;
    }
    let mut bm: BITMAP = std::mem::zeroed();
    let got = GetObjectW(
        info.hbmColor,
        std::mem::size_of::<BITMAP>() as i32,
        &mut bm as *mut _ as *mut _,
    );
    if got == 0 || bm.bmWidth <= 0 || bm.bmHeight <= 0 {
        DeleteObject(info.hbmColor);
        DeleteObject(info.hbmMask);
        return None;
    }
    let (w, h) = (bm.bmWidth as usize, bm.bmHeight as usize);
    let hdc = GetDC(std::ptr::null_mut() as HWND);
    let mem = CreateCompatibleDC(hdc);

    let mut bi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        },
        ..std::mem::zeroed()
    };
    let mut buf = vec![0u8; w * h * 4];
    GetDIBits(
        mem,
        info.hbmColor,
        0,
        h as u32,
        buf.as_mut_ptr() as *mut _,
        &mut bi,
        DIB_RGB_COLORS,
    );
    DeleteDC(mem);
    ReleaseDC(std::ptr::null_mut() as HWND, hdc);
    DeleteObject(info.hbmColor);
    DeleteObject(info.hbmMask);

    // BGRA -> RGBA，处理 alpha 与预乘
    let mut any_alpha = false;
    for px in buf.chunks_exact_mut(4) {
        px.swap(0, 2); // BGRA -> RGBA（交换 B 和 R 通道）
        if px[3] != 0 {
            any_alpha = true;
        }
    }
    if !any_alpha {
        for px in buf.chunks_exact_mut(4) {
            px[3] = 255;
        }
    } else {
        for px in buf.chunks_exact_mut(4) {
            let a = px[3] as u32;
            if a > 0 && a < 255 {
                px[0] = ((px[0] as u32 * 255) / a).min(255) as u8;
                px[1] = ((px[1] as u32 * 255) / a).min(255) as u8;
                px[2] = ((px[2] as u32 * 255) / a).min(255) as u8;
            }
        }
    }
    Some((buf, w, h))
}

#[cfg(not(windows))]
pub fn extract_icon_rgba(_path: &str) -> Option<(Vec<u8>, usize, usize)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use std::fs;

    /// 每个测试独立的临时目录（避免并发测试互相干扰）
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "deskman-test-{}-{}",
            tag,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("创建测试目录失败");
        dir
    }

    fn total_items(config: &AppConfig) -> usize {
        config.groups.iter().map(|group| group.items.len()).sum()
    }

    #[test]
    fn apply_snapshot_adds_new_files_once() {
        let dir = temp_dir("snapshot-add");
        let file = dir.join("a.txt");
        fs::write(&file, b"x").unwrap();
        let mut config = AppConfig::default();
        let base = total_items(&config);

        let outcome = apply_snapshot(&mut config, &[file.clone()]);
        assert_eq!(outcome.added, 1);
        assert_eq!(total_items(&config), base + 1);

        // 同一路径再次同步不会重复插入
        let again = apply_snapshot(&mut config, &[file]);
        assert_eq!(again.added, 0);
        assert_eq!(total_items(&config), base + 1);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_snapshot_skips_ignored_and_system_names() {
        let dir = temp_dir("snapshot-ignore");
        let ignored = dir.join("ignored.txt");
        let desktop_ini = dir.join("desktop.ini");
        fs::write(&ignored, b"x").unwrap();
        fs::write(&desktop_ini, b"x").unwrap();

        let mut config = AppConfig::default();
        assert!(add_ignored(&mut config, &ignored));
        let base = total_items(&config);
        let outcome = apply_snapshot(&mut config, &[ignored.clone(), desktop_ini.clone()]);
        assert_eq!(outcome.added, 0);
        assert_eq!(total_items(&config), base);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ignore_list_is_case_insensitive_and_removable() {
        let mut config = AppConfig::default();
        let path = PathBuf::from(r"C:\Users\Someone\Desktop\Report.TXT");
        assert!(add_ignored(&mut config, &path));
        assert!(!add_ignored(&mut config, &path), "重复加入应返回 false");
        assert!(is_ignored(&config, &path));
        assert!(is_ignored(
            &config,
            Path::new(r"c:\users\someone\desktop\report.txt")
        ));
        assert!(remove_ignored(&mut config, &path));
        assert!(!is_ignored(&config, &path));
    }

    #[test]
    fn snapshot_paths_filters_system_files_and_missing_roots() {
        let dir = temp_dir("snapshot-paths");
        fs::write(dir.join("visible.txt"), b"x").unwrap();
        fs::write(dir.join("desktop.ini"), b"x").unwrap();
        let missing = dir.join("does-not-exist");

        let paths = snapshot_paths(&[dir.clone(), missing]);
        assert_eq!(paths.len(), 1);
        assert!(paths[0].ends_with("visible.txt"));

        let _ = fs::remove_dir_all(&dir);
    }
}
