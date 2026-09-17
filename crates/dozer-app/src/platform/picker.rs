//! macOS 专有文件/目录选择:单入口同时放行文件与目录。

/// macOS 专有：单个"＋"入口要能同时选择**文件**和**目录**（项目文档 /
/// Agent 记忆都能挂任意路径）。rfd 的 `pick_file`/`pick_folder` 各自只允许
/// 一种（`NSOpenPanel` 内部硬编码 `canChooseFiles`/`canChooseDirectories` 二
/// 选一），所以这里直接用 objc2 构造 `NSOpenPanel`，把两个开关都打开，让用
/// 户既能选中文件也能选中文件夹；选中结果按原路径上的 `is_dir()` 判定虚拟链
/// 接类别。只在 macOS 编译——其它平台回退到 rfd 的 `pick_file`（见调用处）。
#[cfg(target_os = "macos")]
pub(crate) fn pick_file_or_dir(start_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::{NSModalResponseOK, NSOpenPanel};
    use objc2_foundation::{NSString, NSURL};

    autoreleasepool(|_| {
        // `Message::ProjectLinkPick` 在 winit 事件循环（macOS 主线程）里同步
        // 处理，此处必然持有主线程标记，可直接同步 runModal。
        let mtm = unsafe { objc2::MainThreadMarker::new_unchecked() };
        let panel = NSOpenPanel::openPanel(mtm);
        // 同时放行文件和目录；保持单选的既有行为（对应 rfd 的 `pick_file`）。
        panel.setCanChooseFiles(true);
        panel.setCanChooseDirectories(true);
        panel.setAllowsMultipleSelection(false);
        if !start_dir.as_os_str().is_empty() {
            let dir = NSString::from_str(&start_dir.to_string_lossy());
            panel.setDirectoryURL(Some(&NSURL::fileURLWithPath(&dir)));
        }
        if panel.runModal() != NSModalResponseOK {
            return None;
        }
        // 单选面板：取第一个 URL 的路径。
        let url = panel.URLs().firstObject()?;
        let path = url.path()?;
        Some(std::path::PathBuf::from(path.to_string()))
    })
}
