//! Tiny compile-time checked translation table.
//!
//! Every user-visible string lives here. To add a language, add a variant to
//! `Lang` and extend the `match` inside the macro. Missing keys are impossible
//! because both languages are required for every entry.

use crate::config::LanguageSetting;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    Zh,
}

impl Lang {
    /// Resolve the language setting, falling back to the Windows UI language.
    pub fn detect(setting: LanguageSetting) -> Lang {
        match setting {
            LanguageSetting::English => Lang::En,
            LanguageSetting::Chinese => Lang::Zh,
            LanguageSetting::Auto => {
                let locale = sys_locale::get_locale()
                    .unwrap_or_else(|| "en".to_string())
                    .to_lowercase();
                if locale.starts_with("zh") {
                    Lang::Zh
                } else {
                    Lang::En
                }
            }
        }
    }
}

macro_rules! strings {
    ($($name:ident => ($en:expr, $zh:expr)),* $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Key {
            $($name),*
        }

        impl Key {
            pub fn tr(self, lang: Lang) -> &'static str {
                match self {
                    $(Self::$name => match lang {
                        Lang::En => $en,
                        Lang::Zh => $zh,
                    }),*
                }
            }
        }
    };
}

strings! {
    // ---------------------------------------------------------------- general
    AppTagline => ("Keep every folder in sync.", "让每个文件夹保持同步。"),
    NavTasks => ("Sync tasks", "同步任务"),
    NavHistory => ("History", "历史记录"),
    NavConflicts => ("Conflicts", "冲突"),
    NavSettings => ("Settings", "设置"),
    NavAbout => ("About", "关于"),
    GlobalAllSyncs => ("All syncs", "全部同步"),
    GlobalActive => ("Active", "运行中"),
    GlobalPaused => ("Paused", "已暂停"),
    StatusAllGood => ("Everything is in sync", "所有内容均已同步"),
    StatusPaused => ("Automatic syncing is paused", "自动同步已暂停"),
    StatusRunning => ("{n} syncing now", "{n} 个任务正在同步"),
    StatusIssues => ("Needs attention: {n}", "需要处理：{n}"),
    OnConflict => ("On conflict: {policy}", "冲突时：{policy}"),
    Close => ("Close", "关闭"),
    DeleteWord => ("Delete", "删除"),
    Cancel => ("Cancel", "取消"),
    Unknown => ("Unknown", "未知"),
    Browse => ("Browse…", "浏览…"),
    SelectFolder => ("Select a folder", "选择文件夹"),
    CopyPath => ("Copy path", "复制路径"),
    PathCopied => ("Path copied to clipboard", "路径已复制到剪贴板"),
    OpenFailed => ("Could not open this location.", "无法打开此位置。"),
    SearchPlaceholder => ("Search history…", "搜索历史记录…"),
    NoMatches => ("No matches", "没有匹配项"),

    // ------------------------------------------------------------------ tasks
    TasksTitle => ("Sync tasks", "同步任务"),
    TasksSubtitle => ("Folders that stay identical, automatically.", "自动保持完全一致的文件文件夹。"),
    NewSync => ("New sync", "新建同步"),
    AddFirstSync => ("Create your first sync", "创建第一个同步任务"),
    EmptyTitle => ("Nothing is syncing yet", "还没有同步任务"),
    EmptyDesc => ("Pick a source folder and a destination. Syncora keeps them in sync quietly in the background.", "选择一个来源文件夹和一个目标文件夹，Syncora 会在后台安静地保持同步。"),
    EmptyStep1Title => ("Choose two folders", "选择两个文件夹"),
    EmptyStep1Desc => ("One source, one target. Any local or network drive works.", "一个来源，一个目标。支持本地磁盘和网络磁盘。"),
    EmptyStep2Title => ("Pick a sync mode", "选择同步模式"),
    EmptyStep2Desc => ("Back up one way, merge both ways, or mirror exactly.", "单向备份、双向合并，或完全镜像。"),
    EmptyStep3Title => ("Let it run", "让它自动运行"),
    EmptyStep3Desc => ("Syncora watches for changes and works from the tray.", "Syncora 会监视变更，并在托盘中持续工作。"),
    Source => ("Source", "来源"),
    Target => ("Target", "目标"),
    SyncNow => ("Sync now", "立即同步"),
    Syncing => ("Syncing…", "同步中…"),
    Scanning => ("Scanning…", "正在扫描…"),
    Queued => ("Queued", "等待中"),
    Edit => ("Edit", "编辑"),
    Delete => ("Delete", "删除"),
    OpenSource => ("Open source", "打开来源"),
    Enabled => ("Enabled", "已启用"),
    PausedLabel => ("Paused", "已暂停"),
    Never => ("Never", "从未"),
    LastSync => ("Last sync", "上次同步"),
    ManualOnly => ("Manual only", "仅手动"),
    UpToDate => ("Up to date", "已是最新"),
    TaskNotSynced => ("Not synced yet", "尚未同步"),
    DeleteTaskTitle => ("Delete this sync task?", "删除此同步任务？"),
    DeleteTaskDesc => ("Only the task is removed. Your files are never touched.", "只会删除任务，不会删除或修改您的文件。"),
    Watching => ("Watching for changes", "正在监视变更"),
    TipSyncNow => ("Scan both folders now and apply any changes.", "立即扫描两个文件夹并应用变更。"),
    TipCancel => ("Stop after the current file. Completed changes are not rolled back.", "当前文件完成后停止。已完成的变更不会回滚。"),
    TipTaskToggle => ("Turn automatic syncing on or off for this task only.", "仅为此任务开启或关闭自动同步。"),
    TipEdit => ("Change folders, mode, schedule and exclusions.", "修改文件夹、模式、计划和排除规则。"),
    TipDelete => ("Remove this task. Files are not deleted.", "移除此任务，不会删除文件。"),
    TipOpenSource => ("Open the source folder in File Explorer.", "在文件资源管理器中打开来源文件夹。"),
    TipOpenTarget => ("Open the target folder in File Explorer.", "在文件资源管理器中打开目标文件夹。"),
    TipAddTask => ("Create a new pair of folders to keep in sync.", "创建一对需要保持同步的文件夹。"),
    TipGlobalSwitch => ("Pause or resume all automatic syncing. Manual syncs are disabled while paused.", "暂停或恢复所有自动同步。暂停时手动同步也会被禁用。"),
    TipModeChip => ("This task's sync mode. Open the task to change it.", "此任务的同步模式。打开任务即可修改。"),
    TipStatusDot => ("Current state of this task.", "此任务的当前状态。"),
    Preview => ("Preview", "预览"),
    TipPreview => ("See what would change without touching any file.", "在不修改任何文件的情况下查看将会发生的变化。"),

    // ------------------------------------------------------------------ modes
    ModeOneWay => ("One-way", "单向"),
    ModeTwoWay => ("Two-way", "双向"),
    ModeMirror => ("Mirror", "镜像"),
    ModeOneWayShort => ("Backup copy", "备份复制"),
    ModeTwoWayShort => ("Both directions", "双向更新"),
    ModeMirrorShort => ("Exact copy", "完全一致"),
    ModeOneWayDesc => ("Copies new and changed files from source to target. Nothing is deleted from the target.", "将来源端新增或修改的文件复制到目标端，目标端不会删除任何内容。"),
    ModeTwoWayDesc => ("Changes on either side flow to the other. Deletions are propagated too.", "任一侧的变更都会同步到另一侧，删除操作也会同步。"),
    ModeMirrorDesc => ("Makes the target an exact copy of the source, including deletions.", "让目标端与来源端完全一致，包括删除操作。"),
    MirrorWarning => ("Mirror deletes extra files in the target. Double-check that the target folder is correct.", "镜像模式会删除目标端多余的文件，请确认目标文件夹正确。"),
    TwoWayWarning => ("On the first two-way sync, files that exist on both sides with different content become conflicts.", "首次双向同步时，两侧同名但内容不同的文件会被视为冲突。"),
    WatchLabel => ("Watch for changes in real time", "实时监视变更"),
    WatchHelp => ("Syncs a few seconds after a file is added, changed or deleted. Recommended.", "文件新增、修改或删除后几秒内自动同步。推荐开启。"),

    // ------------------------------------------------------------ task editor
    NewTaskTitle => ("New sync task", "新建同步任务"),
    EditTaskTitle => ("Edit sync task", "编辑同步任务"),
    FieldName => ("Task name", "任务名称"),
    FieldNameHint => ("e.g. Documents backup", "例如：文档备份"),
    FieldSource => ("Source folder", "来源文件夹"),
    FieldTarget => ("Target folder", "目标文件夹"),
    PathHint => ("Tip: you can also paste a path directly.", "提示：也可以直接粘贴路径。"),
    ModeLabel => ("Sync mode", "同步模式"),
    ModeHelp => ("How changes travel between the two folders.", "变更如何在两个文件夹之间传递。"),
    Automation => ("Automation", "自动化"),
    IntervalLabel => ("Also check periodically", "定时检查"),
    IntervalHelp => ("A periodic scan catches changes that real-time watching can miss, such as network drives.", "定期扫描可以捕捉实时监视可能遗漏的变更，例如网络磁盘。"),
    Advanced => ("Advanced options", "高级选项"),
    ExcludesLabel => ("Exclude patterns", "排除规则"),
    ExcludesHelp => ("One glob per line, matched against relative paths. Examples: *.tmp, cache/**, **/node_modules.", "每行一个通配符，匹配相对路径。例如：*.tmp、cache/**、**/node_modules。"),
    ConflictLabel => ("When both sides change the same file", "当两侧同时修改同一文件时"),
    ConflictInherit => ("Use global setting ({policy})", "使用全局设置（{policy}）"),
    ConflictInheritShort => ("Use global setting", "使用全局设置"),
    VerifyLabel => ("Verify copies with checksums", "使用校验和验证副本"),
    VerifyHelp => ("Reads every copied file back to confirm it matches. Slower, but safer for important backups.", "复制后重新读取文件以确认内容一致。速度较慢，但重要备份更安全。"),
    CreateTask => ("Create task", "创建任务"),
    SaveChanges => ("Save changes", "保存更改"),
    ValidationName => ("Give this task a name.", "请为任务填写名称。"),
    ValidationSource => ("Choose a source folder.", "请选择来源文件夹。"),
    ValidationTarget => ("Choose a target folder.", "请选择目标文件夹。"),
    ValidationSame => ("Source and target must be different folders.", "来源和目标不能是同一个文件夹。"),
    ValidationNested => ("Target cannot be inside the source (or the other way around).", "目标不能位于来源内部（反之亦然）。"),
    ValidationSourceMissing => ("The source folder does not exist.", "来源文件夹不存在。"),
    ValidationSourceIsFile => ("The source path is a file, not a folder.", "来源路径是文件，不是文件夹。"),
    ValidationTargetIsFile => ("The target path is a file, not a folder.", "目标路径是文件，不是文件夹。"),

    // ---------------------------------------------------------------- settings
    SettingsTitle => ("Settings", "设置"),
    SettingsSubtitle => ("Make Syncora work the way you do.", "让 Syncora 按照您的方式工作。"),
    SectionGeneral => ("General", "常规"),
    Language => ("Language", "语言"),
    LanguageAuto => ("System default", "跟随系统"),
    LanguageEnglish => ("English", "English"),
    LanguageChinese => ("简体中文", "简体中文"),
    LanguageHelp => ("On first launch Syncora follows the Windows display language.", "首次启动时，Syncora 会跟随 Windows 显示语言。"),
    Appearance => ("Appearance", "外观"),
    ThemeSystem => ("Follow system", "跟随系统"),
    ThemeLight => ("Light", "浅色"),
    ThemeDark => ("Dark", "深色"),
    ThemeHelp => ("Follows Windows light/dark mode until you pick one manually.", "默认跟随 Windows 浅色/深色模式，也可以手动选择。"),
    SectionStartup => ("Startup & window", "启动与窗口"),
    StartWithWindows => ("Start Syncora when Windows starts", "开机时自动启动 Syncora"),
    StartWithWindowsHelp => ("Runs in the background. No administrator rights needed.", "在后台运行，无需管理员权限。"),
    StartMinimized => ("Start minimized to the tray", "启动时最小化到托盘"),
    StartMinimizedHelp => ("Syncora begins quietly without opening a window.", "Syncora 会在后台安静启动，不打开窗口。"),
    CloseToTray => ("Closing the window keeps Syncora running", "关闭窗口后继续在托盘中运行"),
    CloseToTrayHelp => ("Turn this off to quit completely when the window is closed.", "关闭此项后，关闭窗口将完全退出。"),
    SectionSync => ("Sync behavior", "同步行为"),
    DefaultInterval => ("Default check interval for new tasks", "新任务的默认检查间隔"),
    DefaultConflict => ("Default conflict handling", "默认冲突处理"),
    ConflictAsk => ("Ask me", "询问我"),
    ConflictKeepBoth => ("Keep both versions", "保留两个版本"),
    ConflictPreferSource => ("Source wins", "以来源为准"),
    ConflictPreferTarget => ("Target wins", "以目标为准"),
    ConflictSkip => ("Skip until resolved", "暂时跳过"),
    ConflictHelp => ("Used when both sides changed the same file since the last sync.", "当两侧自上次同步后都修改了同一文件时使用。"),
    Notifications => ("Show Windows notifications", "显示 Windows 通知"),
    NotificationsHelp => ("Alerts you about conflicts and errors, even when the window is hidden.", "即使窗口隐藏，也会提醒冲突和错误。"),
    NotifySuccess => ("Notify me when a sync finishes", "同步完成后通知我"),
    SkipHidden => ("Skip hidden and system files", "跳过隐藏和系统文件"),
    SkipHiddenHelp => ("Useful for system folders. Off by default so backups stay complete.", "适用于系统文件夹。默认关闭以确保备份完整。"),
    MaxHistory => ("History entries to keep", "保留的历史记录数量"),
    MaxHistoryHelp => ("Older entries are removed automatically. Between 100 and 5000.", "较旧的记录会自动删除，范围 100 到 5000。"),
    SectionData => ("Data", "数据"),
    ExportSettings => ("Export settings…", "导出设置…"),
    ImportSettings => ("Import settings…", "导入设置…"),
    ExportHelp => ("Saves all tasks and preferences to a JSON file.", "将所有任务和偏好设置保存为 JSON 文件。"),
    ImportHelp => ("Replaces your current tasks and preferences.", "将替换当前的任务和偏好设置。"),
    OpenDataFolder => ("Open data folder", "打开数据文件夹"),
    OpenLog => ("Open log file", "打开日志文件"),
    OpenLogHelp => ("Available after Syncora has written its first log entry.", "Syncora 写入首条日志后即可打开。"),
    ResetSettings => ("Reset all settings", "重置所有设置"),
    ResetTitle => ("Reset Syncora?", "重置 Syncora？"),
    ResetDesc => ("This removes all sync tasks, history and preferences. Your files are not touched.", "这将删除所有同步任务、历史记录和偏好设置，不会影响您的文件。"),
    ResetWord => ("Reset everything", "全部重置"),
    AboutDesc => ("A quiet little app that keeps your folders identical.", "一个安静的小工具，让您的文件夹保持一致。"),
    VersionLabel => ("Version", "版本"),
    BuiltWith => ("Built with Rust and egui. Your files never leave your computer.", "使用 Rust 和 egui 构建。您的文件不会离开本机。"),
    LicenseNote => ("Free to use, no account, no telemetry.", "免费使用，无需账号，无遥测。"),
    AboutSource => ("Source code:", "源代码："),
    ImportConfirmTitle => ("Replace current settings?", "替换当前设置？"),
    ImportConfirmDesc => ("This will replace all tasks and preferences with the contents of the selected file. Your files are not touched.", "这将用所选文件的内容替换所有任务和偏好设置，不会影响您的文件。"),
    ImportWord => ("Import", "导入"),

    // ----------------------------------------------------------------- history
    HistoryTitle => ("Sync history", "同步历史"),
    HistorySubtitle => ("A record of every sync, including what changed.", "每次同步的记录，包括变更内容。"),
    HistoryEmpty => ("No syncs yet", "还没有同步记录"),
    HistoryEmptyDesc => ("Run a task and its results will appear here.", "运行任务后，结果会显示在这里。"),
    ClearHistory => ("Clear history", "清空历史"),
    ClearHistoryTitle => ("Clear all history?", "清空所有历史记录？"),
    ClearHistoryDesc => ("This only removes the log. Your files and tasks are kept.", "只会删除记录，您的文件和任务都会保留。"),
    AllTasks => ("All tasks", "所有任务"),
    AllResults => ("All results", "所有结果"),
    Duration => ("Duration", "耗时"),
    Transferred => ("Transferred", "传输"),
    ShowingLatest => ("Showing the latest {n} entries.", "显示最近 {n} 条记录。"),
    HistorySummary => ("{copied} copied · {updated} updated · {deleted} deleted · {conflicts} conflicts", "复制 {copied} · 更新 {updated} · 删除 {deleted} · 冲突 {conflicts}"),
    ResultSuccess => ("Success", "成功"),
    ResultPartial => ("Completed with issues", "部分完成"),
    ResultFailed => ("Failed", "失败"),
    ResultCancelled => ("Cancelled", "已取消"),

    // --------------------------------------------------------------- conflicts
    ConflictsTitle => ("Conflicts", "冲突"),
    ConflictsSubtitle => ("The same file changed in both places. Choose which version to keep.", "同一文件在两侧都被修改。请选择要保留的版本。"),
    ConflictsEmpty => ("No conflicts", "没有冲突"),
    ConflictsEmptyDesc => ("When both sides change the same file, it will show up here.", "当两侧同时修改同一文件时，会显示在这里。"),
    ConflictSourceVersion => ("Source version", "来源版本"),
    ConflictTargetVersion => ("Target version", "目标版本"),
    KeepSource => ("Keep source", "保留来源"),
    KeepTarget => ("Keep target", "保留目标"),
    KeepBoth => ("Keep both", "两者都保留"),
    ConflictResolveHelp => ("The chosen version is copied to both folders.", "所选版本会复制到两个文件夹。"),
    ConflictKeepBothHelp => ("Both versions are kept. The older one is renamed with a conflict suffix.", "两个版本都会保留，较旧的版本会加上冲突后缀重命名。"),
    UnresolvedCount => ("{n} unresolved", "{n} 个未解决"),
    Resolve => ("Resolve", "解决"),
    ConflictResolvedToast => ("Conflict resolved", "冲突已解决"),
    ConflictResolveFailed => ("Could not resolve the conflict: {error}", "无法解决冲突：{error}"),
    ConflictSourceMissing => ("No longer in source", "来源中已不存在"),
    ConflictTargetMissing => ("No longer in target", "目标中已不存在"),

    // ----------------------------------------------------------- notifications
    NotifConflictTitle => ("Sync conflict in “{task}”", "“{task}” 中检测到同步冲突"),
    NotifConflictBody => ("{path} changed in both folders. Open Syncora to choose a version.", "{path} 在两个文件夹中都被修改。打开 Syncora 选择要保留的版本。"),
    NotifErrorTitle => ("Sync failed: {task}", "同步失败：{task}"),
    NotifSuccessTitle => ("Sync complete: {task}", "同步完成：{task}"),
    NotifSuccessBody => ("{copied} copied, {deleted} deleted, {conflicts} conflicts.", "复制 {copied} 项，删除 {deleted} 项，冲突 {conflicts} 项。"),

    // -------------------------------------------------------------------- tray
    TrayShow => ("Show Syncora", "显示 Syncora"),
    TraySyncAll => ("Sync all now", "立即同步全部"),
    TrayPauseAll => ("Pause all syncs", "暂停全部同步"),
    TrayResumeAll => ("Resume all syncs", "恢复全部同步"),
    TraySettings => ("Settings…", "设置…"),
    TrayExit => ("Quit Syncora", "退出 Syncora"),
    TrayTooltip => ("Syncora — {n} tasks active", "Syncora — {n} 个任务运行中"),
    TrayTooltipPaused => ("Syncora — paused", "Syncora — 已暂停"),

    // ------------------------------------------------------------------ toasts
    ToastTaskCreated => ("Sync task created", "同步任务已创建"),
    ToastTaskSaved => ("Changes saved", "更改已保存"),
    ToastTaskDeleted => ("Task deleted", "任务已删除"),
    ToastSettingsExported => ("Settings exported", "设置已导出"),
    ToastSettingsImported => ("Settings imported", "设置已导入"),
    ToastExportFailed => ("Could not export settings: {error}", "导出设置失败：{error}"),
    ToastImportFailed => ("Could not import settings: {error}", "导入设置失败：{error}"),
    ToastSyncDone => ("“{task}” is up to date", "“{task}” 已是最新"),
    ToastSyncFailed => ("“{task}” could not finish", "“{task}” 未能完成"),
    ToastAutostartFailed => ("Could not update the startup setting: {error}", "无法更新开机启动设置：{error}"),
    ToastNothingToSync => ("Everything is already in sync", "所有内容已同步"),
    ToastConflicts => ("Conflicts found in “{task}” — review them on the Conflicts page", "“{task}” 中发现冲突 — 请在“冲突”页面处理"),
    ToastWatcherWarning => ("Real-time watching is unavailable for “{task}”. Periodic checks will still run.", "“{task}” 的实时监视不可用，仍会按计划检查。"),

    // ------------------------------------------------------------------ errors
    ErrSourceMissing => ("Source folder not found", "找不到来源文件夹"),
    ErrTargetMissing => ("Target folder not found", "找不到目标文件夹"),
    ErrScan => ("Could not scan {path}: {error}", "无法扫描 {path}：{error}"),
    ErrOther => ("Something went wrong: {error}", "出现错误：{error}"),
    AlreadyRunning => (
        "Syncora is already running.\nLook for its icon in the notification area.",
        "Syncora 已在运行。\n请在通知区域中寻找它的图标。"
    ),

    // ----------------------------------------------------------------- preview
    PreviewTitle => ("Preview changes", "预览变更"),
    PreviewDesc => ("Nothing has been changed yet. This is what the next sync would do.", "尚未进行任何变更。以下是下次同步将会执行的操作。"),
    PreviewScanned => ("Scanned {src} source files ({src_size}) and {dst} target files ({dst_size})", "已扫描来源端 {src} 个文件（{src_size}）与目标端 {dst} 个文件（{dst_size}）"),
    PreviewCopy => ("To copy", "待复制"),
    PreviewUpdate => ("To update", "待更新"),
    PreviewDelete => ("To delete", "待删除"),
    PreviewConflicts => ("Conflicts", "冲突"),
    PreviewNone => ("Everything is already in sync.", "所有内容已同步。"),
    PreviewRun => ("Run this sync", "开始同步"),
    PreviewMore => ("…and {n} more", "…还有 {n} 项"),
    PreviewSample => ("A sample of the changes", "变更示例"),
    PreviewScanning => ("Scanning both folders…", "正在扫描两个文件夹…"),
    ProgressFiles => ("{done} / {total}", "{done} / {total}"),

    // -------------------------------------------------------------- time units
    JustNow => ("just now", "刚刚"),
    MinutesAgo => ("{n} min ago", "{n} 分钟前"),
    HoursAgo => ("{n} h ago", "{n} 小时前"),
    DaysAgo => ("{n} d ago", "{n} 天前"),
    Yesterday => ("Yesterday", "昨天"),
    SecondsShort => ("{n}s", "{n} 秒"),
    MinutesShort => ("{n}m", "{n} 分"),
    HoursShort => ("{n}h", "{n} 小时"),
    DaysShort => ("{n}d", "{n} 天"),

    // ---------------------------------------------------------------- intervals
    IntervalManual => ("Manual only", "仅手动"),
    Interval1m => ("Every minute", "每分钟"),
    Interval5m => ("Every 5 minutes", "每 5 分钟"),
    Interval15m => ("Every 15 minutes", "每 15 分钟"),
    Interval30m => ("Every 30 minutes", "每 30 分钟"),
    Interval1h => ("Every hour", "每小时"),
    Interval6h => ("Every 6 hours", "每 6 小时"),
    Interval24h => ("Every day", "每天"),
    IntervalHelpShort => ("How often to run a full comparison.", "多久执行一次完整比较。"),

    // -------------------------------------------------------------------- help
    HelpTitle => ("Quick guide", "快速指南"),
    HelpIntro => ("Syncora keeps two folders identical. Here is all you need to know.", "Syncora 让两个文件夹保持一致。以下是您需要了解的全部内容。"),
    HelpStep1Title => ("1. Create a task", "1. 创建任务"),
    HelpStep1Desc => ("Pick a source and a target folder, then choose how changes should travel.", "选择来源和目标文件夹，然后选择变更的传递方式。"),
    HelpStep2Title => ("2. Let it watch", "2. 自动监视"),
    HelpStep2Desc => ("Syncora runs in the background. Close the window and it keeps working from the tray.", "Syncora 在后台运行。关闭窗口后，它会继续在托盘中工作。"),
    HelpStep3Title => ("3. Review when needed", "3. 需要时查看"),
    HelpStep3Desc => ("History shows what changed. Conflicts wait for your decision unless you set an automatic rule.", "历史记录会显示变更内容。除非设置了自动规则，否则冲突会等待您决定。"),
    HelpConflictTitle => ("Conflicts", "冲突"),
    HelpConflictDesc => ("A conflict means the same file changed on both sides. Keep either version, keep both, or let Syncora apply your default rule.", "冲突表示同一文件在两侧都被修改。您可以保留任一版本、两者都保留，或让 Syncora 应用默认规则。"),
    HelpTrayTitle => ("Tray & startup", "托盘与启动"),
    HelpTrayDesc => ("Closing the window hides Syncora to the notification area. Enable “Start with Windows” in Settings to keep folders in sync after a reboot.", "关闭窗口会将 Syncora 隐藏到通知区域。在设置中开启“开机启动”，重启后也会继续同步。"),
    HelpTip => ("Tip: hover over any ? or button to see what it does.", "提示：将鼠标悬停在任意 ? 或按钮上可查看说明。"),

    // ------------------------------------------------------------------- about
    AboutTitle => ("About Syncora", "关于 Syncora"),
    AboutFeature1 => ("One-way, two-way and mirror sync", "单向、双向和镜像同步"),
    AboutFeature2 => ("Real-time watching with scheduled fallback checks", "实时监视，并以定时检查作为补充"),
    AboutFeature3 => ("Conflict detection with Windows notifications", "冲突检测与 Windows 通知"),
    AboutFeature4 => ("English and Simplified Chinese, light and dark themes", "英文和简体中文，浅色和深色主题"),
    AboutFeature5 => ("Portable single executable, no admin rights required", "单文件便携版，无需管理员权限"),
}

/// Small helper for template strings such as `"{n} tasks"`.
pub fn fill(template: &str, pairs: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (key, value) in pairs {
        out = out.replace(key, value);
    }
    out
}

/// Human-readable name for a conflict policy, used in tooltips and combos.
pub fn policy_label(policy: crate::config::ConflictPolicy, lang: Lang) -> &'static str {
    use crate::config::ConflictPolicy;
    match policy {
        ConflictPolicy::Ask => Key::ConflictAsk.tr(lang),
        ConflictPolicy::KeepBoth => Key::ConflictKeepBoth.tr(lang),
        ConflictPolicy::PreferSource => Key::ConflictPreferSource.tr(lang),
        ConflictPolicy::PreferTarget => Key::ConflictPreferTarget.tr(lang),
        ConflictPolicy::Skip => Key::ConflictSkip.tr(lang),
    }
}
