use serde::{Deserialize, Serialize};

macro_rules! keys {
    ($($key:ident),* $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Key {
            $($key),*
        }

        impl Key {
            /// Every localizable key, in declaration order. Used by tests to
            /// guarantee that each language provides a value for each key.
            #[cfg(test)]
            pub const ALL: &'static [Key] = &[$(Key::$key),*];
        }
    };
}

keys! {
    RecordingSection,
    MillisecondsSuffix,
    File,
    View,
    Help,
    About,
    Exit,
    Repository,
    OpenSession,
    OpenSessionFolder,
    RecentSessions,
    NoRecentSessions,
    RecentSessionMissing,
    Appearance,
    System,
    Light,
    Dark,
    Font,
    Fonts,
    SearchFonts,
    FontSearchNoResults,
    FontListLoading,
    Bytes,
    Kib,
    Mib,
    DefaultFont,
    FontSize,
    Language,
    English,
    Japanese,
    SimplifiedChinese,
    WaitingForSai,
    CheckingCompatibility,
    CompatibilityExact,
    CompatibilityValidated,
    CompatibilityUnsupported,
    NeedCanvas,
    ErrorProcessAccess,
    ErrorNoCanvas,
    SelectCanvas,
    Canvas,
    Session,
    SessionHint,
    Tracked,
    Captures,
    Size,
    StartRecording,
    StopRecording,
    Recording,
    Stopping,
    Advanced,
    IdleDelay,
    IdleDelayTip,
    MinCaptureGap,
    MinCaptureGapTip,
    MaxActiveGap,
    MaxActiveGapTip,
    ZstdLevel,
    ZstdLevelTip,
    CheckpointInterval,
    CheckpointIntervalTip,
    CheckpointRatio,
    CheckpointRatioTip,
    InputWatcher,
    InputWatcherTip,
    Export,
    Pace,
    PaceTip,
    CapturesPerSecond,
    CustomPace,
    Fps,
    Quality,
    Draft,
    Balanced,
    High,
    Archival,
    Encoder,
    EncoderTip,
    Auto,
    Nvenc,
    X264,
    Resolution,
    P720,
    P1080,
    P1440,
    P2160,
    P75,
    P50,
    P25,
    CanvasMode,
    MaxCanvas,
    InitialCanvas,
    HoldEnd,
    PreviewEnd,
    SecondsSuffix,
    SkipProvisional,
    Duration,
    ExportVideo,
    Exporting,
    ExportComplete,
    Planning,
    RefreshPreview,
    UnavailableValue,
    Loading,
    PreviewUnavailable,
    PreviewEmpty,
    PreviewRefreshTip,
    TooltipRefresh,
    ChooseDirectory,
    ChooseOutput,
    AboutTitle,
    ApplicationName,
    Version,
    AboutDescription,
    AboutLicense,
    AboutFree,
    AboutDistribution,
    FinishingBeforeExit,
    Mp4,
    Gib,
    PointSuffix,
    PercentSuffix,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[default]
    English,
    Japanese,
    SimplifiedChinese,
}

impl Language {
    #[cfg(test)]
    pub const ALL: [Language; 3] = [
        Language::English,
        Language::Japanese,
        Language::SimplifiedChinese,
    ];
}

/// Copyable translation context. Widgets take one of these instead of a raw
/// language parameter so call sites read as `i18n.tr(Key::RecordingSection)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Translator {
    language: Language,
}

impl Translator {
    pub const fn new(language: Language) -> Self {
        Self { language }
    }

    pub const fn language(self) -> Language {
        self.language
    }

    pub fn tr(self, key: Key) -> &'static str {
        match self.language {
            Language::English => english(key),
            Language::Japanese => japanese(key),
            Language::SimplifiedChinese => simplified_chinese(key),
        }
    }

    pub fn size(self, bytes: u64) -> String {
        format_size(self, bytes)
    }

    pub fn tracked(self, ms: u64) -> String {
        format_tracked(self, ms)
    }
}

fn english(key: Key) -> &'static str {
    use Key::*;
    match key {
        RecordingSection => "Recording",
        MillisecondsSuffix => " ms",
        File => "File",
        View => "View",
        Help => "Help",
        About => "About",
        Exit => "Exit",
        Repository => "GitHub Repository",
        OpenSession => "Open session…",
        OpenSessionFolder => "Open session folder",
        RecentSessions => "Recent sessions",
        NoRecentSessions => "No recent sessions",
        RecentSessionMissing => "Session folder is no longer available",
        Appearance => "Appearance",
        System => "System",
        Light => "Light",
        Dark => "Dark",
        Font => "Font",
        Fonts => "Fonts…",
        SearchFonts => "Search fonts",
        FontSearchNoResults => "No matching fonts",
        FontListLoading => "Loading fonts…",
        Bytes => "B",
        Kib => "KiB",
        Mib => "MiB",
        DefaultFont => "Default",
        FontSize => "Font size",
        Language => "Language",
        English => "English",
        Japanese => "日本語",
        SimplifiedChinese => "简体中文",
        WaitingForSai => "Waiting for SAI…",
        CheckingCompatibility => "Checking compatibility…",
        CompatibilityExact => "SAI2 detected · Supported build",
        CompatibilityValidated => "Compatible build detected",
        CompatibilityUnsupported => "Unsupported build",
        NeedCanvas => "Open a canvas to finish compatibility checking",
        ErrorProcessAccess => "Could not access the SAI process.",
        ErrorNoCanvas => "No canvas is available. Open a document in SAI and refresh.",
        SelectCanvas => "Select a canvas",
        Canvas => "Canvas",
        Session => "Session",
        SessionHint => "New session: recordings/session-<timestamp>",
        Tracked => "Tracked",
        Captures => "Captures",
        Size => "Size",
        StartRecording => "Start recording",
        StopRecording => "Stop recording",
        Recording => "Recording…",
        Stopping => "Stopping…",
        Advanced => "Advanced",
        IdleDelay => "Idle delay",
        IdleDelayTip => "Wait this long after input stops before saving a capture.",
        MinCaptureGap => "Minimum capture gap",
        MinCaptureGapTip => "Minimum time between saved captures.",
        MaxActiveGap => "Maximum active gap",
        MaxActiveGapTip => "Force a capture after this interval during continuous drawing.",
        ZstdLevel => "Zstd level",
        ZstdLevelTip => "Compression level used for saved capture data.",
        CheckpointInterval => "Checkpoint interval",
        CheckpointIntervalTip => {
            "Write a recovery checkpoint after this many events (0 disables this limit)."
        }
        CheckpointRatio => "Checkpoint changed ratio",
        CheckpointRatioTip => "Write a checkpoint when this fraction of the canvas changes.",
        InputWatcher => "Input watcher",
        InputWatcherTip => "Use input activity to schedule captures.",
        Export => "Export",
        Pace => "Pace",
        PaceTip => "How many recorded visual states are played each second.",
        CapturesPerSecond => "captures/s",
        CustomPace => "Custom pace",
        Fps => "FPS",
        Quality => "Quality",
        Draft => "Draft",
        Balanced => "Balanced",
        High => "High",
        Archival => "Archival",
        Encoder => "Encoder",
        EncoderTip => {
            "Auto probes NVIDIA NVENC and falls back to software x264 when unavailable. FFmpeg must be installed."
        }
        Auto => "Auto",
        Nvenc => "NVENC",
        X264 => "x264",
        Resolution => "Resolution",
        P720 => "720p",
        P1080 => "1080p",
        P1440 => "1440p",
        P2160 => "2160p",
        P75 => "75%",
        P50 => "50%",
        P25 => "25%",
        CanvasMode => "Canvas mode",
        MaxCanvas => "Maximum canvas size",
        InitialCanvas => "Initial canvas",
        HoldEnd => "Hold at end",
        PreviewEnd => "Opening artwork preview",
        SecondsSuffix => " s",
        SkipProvisional => "Skip provisional captures",
        Duration => "Video length",
        ExportVideo => "Export video",
        Exporting => "Exporting…",
        ExportComplete => "Export complete",
        Planning => "Preparing export…",
        RefreshPreview => "Refresh preview",
        UnavailableValue => "—",
        Loading => "Loading…",
        PreviewUnavailable => "Preview unavailable",
        PreviewEmpty => "Select a canvas or open a session to see a preview.",
        PreviewRefreshTip => {
            "Read the current canvas or final session image once. During recording, this does not save a capture or change the timeline or statistics."
        }
        TooltipRefresh => "Refresh the SAI process and canvas list.",
        ChooseDirectory => "Choose folder…",
        ChooseOutput => "Save video",
        AboutTitle => "About timelapse4sai2",
        ApplicationName => super::APP_NAME,
        Version => "Version",
        AboutDescription => "Record and export PaintTool SAI 2 canvas timelapses.",
        AboutLicense => "License: GNU General Public License v3.0 only (GPL-3.0-only).",
        AboutFree => "Free and open source software.",
        AboutDistribution => {
            "This software is available for free at the linked repository. If you paid for it, you were scammed."
        }
        FinishingBeforeExit => "Finishing before exit…",
        Mp4 => "MP4",
        Gib => "GiB",
        PointSuffix => "pt",
        PercentSuffix => "%",
    }
}

fn simplified_chinese(key: Key) -> &'static str {
    use Key::*;
    match key {
        RecordingSection => "录制",
        MillisecondsSuffix => " 毫秒",
        File => "文件",
        View => "视图",
        Help => "帮助",
        About => "关于",
        Exit => "退出",
        Repository => "GitHub 仓库",
        OpenSession => "打开会话…",
        OpenSessionFolder => "打开会话文件夹",
        RecentSessions => "最近的会话",
        NoRecentSessions => "没有最近的会话",
        RecentSessionMissing => "会话文件夹已不存在",
        Appearance => "外观",
        System => "跟随系统",
        Light => "浅色",
        Dark => "深色",
        Font => "字体",
        Fonts => "字体…",
        SearchFonts => "搜索字体",
        FontSearchNoResults => "没有匹配的字体",
        FontListLoading => "正在加载字体…",
        Bytes => "B",
        Kib => "KiB",
        Mib => "MiB",
        DefaultFont => "默认",
        FontSize => "字号",
        Language => "语言",
        English => "English",
        Japanese => "日本語",
        SimplifiedChinese => "简体中文",
        WaitingForSai => "正在等待 SAI…",
        CheckingCompatibility => "正在检查兼容性…",
        CompatibilityExact => "已检测到 SAI2 · 支持此版本",
        CompatibilityValidated => "检测到兼容版本",
        CompatibilityUnsupported => "不支持此版本",
        NeedCanvas => "请打开一个画布以完成兼容性检查",
        ErrorProcessAccess => "无法访问 SAI 进程。",
        ErrorNoCanvas => "没有可用的画布。请在 SAI 中打开文档并刷新。",
        SelectCanvas => "选择画布",
        Canvas => "画布",
        Session => "会话",
        SessionHint => "新会话：recordings/session-<timestamp>",
        Tracked => "记录时长",
        Captures => "捕获数",
        Size => "大小",
        StartRecording => "开始录制",
        StopRecording => "停止录制",
        Recording => "录制中…",
        Stopping => "正在停止…",
        Advanced => "高级",
        IdleDelay => "空闲延迟",
        IdleDelayTip => "输入停止后等待这段时间再保存捕获。",
        MinCaptureGap => "最短捕获间隔",
        MinCaptureGapTip => "两次保存捕获之间的最短时间。",
        MaxActiveGap => "最长活动间隔",
        MaxActiveGapTip => "连续绘制时，超过此间隔后强制保存一次捕获。",
        ZstdLevel => "Zstd 级别",
        ZstdLevelTip => "保存捕获数据时使用的压缩级别。",
        CheckpointInterval => "检查点间隔",
        CheckpointIntervalTip => "每记录这么多事件后写入一个恢复检查点（0 表示不限制）。",
        CheckpointRatio => "检查点变化比例",
        CheckpointRatioTip => "当画布变化达到此比例时写入检查点。",
        InputWatcher => "输入监视",
        InputWatcherTip => "根据输入活动安排捕获。",
        Export => "导出",
        Pace => "回放速率",
        PaceTip => "每秒播放多少个已记录的画面状态。",
        CapturesPerSecond => "次捕获/秒",
        CustomPace => "自定义回放速率",
        Fps => "FPS",
        Quality => "质量",
        Draft => "草稿",
        Balanced => "均衡",
        High => "高",
        Archival => "存档",
        Encoder => "编码器",
        EncoderTip => "自动检测 NVIDIA NVENC，不可用时回退到软件 x264。需要安装 FFmpeg。",
        Auto => "自动",
        Nvenc => "NVENC",
        X264 => "x264",
        Resolution => "分辨率",
        P720 => "720p",
        P1080 => "1080p",
        P1440 => "1440p",
        P2160 => "2160p",
        P75 => "75%",
        P50 => "50%",
        P25 => "25%",
        CanvasMode => "画布模式",
        MaxCanvas => "最大画布尺寸",
        InitialCanvas => "初始画布",
        HoldEnd => "结尾停留",
        PreviewEnd => "展示作品预览",
        SecondsSuffix => " 秒",
        SkipProvisional => "跳过临时捕获",
        Duration => "视频时长",
        ExportVideo => "导出视频",
        Exporting => "正在导出…",
        ExportComplete => "导出完成",
        Planning => "正在准备导出…",
        RefreshPreview => "刷新预览",
        UnavailableValue => "—",
        Loading => "正在加载…",
        PreviewUnavailable => "预览不可用",
        PreviewEmpty => "选择一个画布或打开一个会话以查看预览。",
        PreviewRefreshTip => "读取一次当前画布或会话最终图像。录制期间不会保存捕获，也不会改变时间线或统计。",
        TooltipRefresh => "刷新 SAI 进程和画布列表。",
        ChooseDirectory => "选择文件夹…",
        ChooseOutput => "保存视频",
        AboutTitle => "关于 timelapse4sai2",
        ApplicationName => super::APP_NAME,
        Version => "版本",
        AboutDescription => "录制并导出 PaintTool SAI 2 画布的延时视频。",
        AboutLicense => "许可证：GNU 通用公共许可证第 3 版（仅限 GPL-3.0-only）。",
        AboutFree => "自由且开源的软件。",
        AboutDistribution => "本软件可在链接的仓库免费获取。如果你付费获取了这个软件，那你就被骗了。",
        FinishingBeforeExit => "正在完成收尾工作并退出…",
        Mp4 => "MP4",
        Gib => "GiB",
        PointSuffix => "pt",
        PercentSuffix => "%",
    }
}

fn japanese(key: Key) -> &'static str {
    use Key::*;
    match key {
        RecordingSection => "録画",
        MillisecondsSuffix => " ms",
        File => "ファイル",
        View => "表示",
        Help => "ヘルプ",
        About => "このアプリについて",
        Exit => "終了",
        Repository => "GitHub リポジトリ",
        OpenSession => "セッションを開く…",
        OpenSessionFolder => "セッションフォルダーを開く",
        RecentSessions => "最近のセッション",
        NoRecentSessions => "最近のセッションはありません",
        RecentSessionMissing => "セッションフォルダーが見つかりません",
        Appearance => "外観",
        System => "システム",
        Light => "ライト",
        Dark => "ダーク",
        Font => "フォント",
        Fonts => "フォント…",
        SearchFonts => "フォントを検索",
        FontSearchNoResults => "一致するフォントがありません",
        FontListLoading => "フォントを読み込み中…",
        Bytes => "B",
        Kib => "KiB",
        Mib => "MiB",
        DefaultFont => "既定",
        FontSize => "フォントサイズ",
        Language => "言語",
        English => "English",
        Japanese => "日本語",
        SimplifiedChinese => "简体中文",
        WaitingForSai => "SAI を待機しています…",
        CheckingCompatibility => "互換性を確認しています…",
        CompatibilityExact => "SAI2 を検出しました · 対応バージョン",
        CompatibilityValidated => "互換性のあるバージョンを検出しました",
        CompatibilityUnsupported => "未対応のバージョンです",
        NeedCanvas => "互換性の確認を完了するにはキャンバスを開いてください",
        ErrorProcessAccess => "SAI プロセスにアクセスできませんでした。",
        ErrorNoCanvas => "利用できるキャンバスがありません。SAI でドキュメントを開いて更新してください。",
        SelectCanvas => "キャンバスを選択",
        Canvas => "キャンバス",
        Session => "セッション",
        SessionHint => "新規セッション: recordings/session-<timestamp>",
        Tracked => "記録時間",
        Captures => "キャプチャ数",
        Size => "サイズ",
        StartRecording => "録画開始",
        StopRecording => "録画停止",
        Recording => "録画中…",
        Stopping => "停止しています…",
        Advanced => "詳細設定",
        IdleDelay => "待機時間",
        IdleDelayTip => "入力が止まってからキャプチャを保存するまでの待ち時間です。",
        MinCaptureGap => "最小キャプチャ間隔",
        MinCaptureGapTip => "キャプチャを保存する間隔の最小値です。",
        MaxActiveGap => "最大動作間隔",
        MaxActiveGapTip => "描画が続く間、この間隔を超えたらキャプチャを強制保存します。",
        ZstdLevel => "Zstd レベル",
        ZstdLevelTip => "キャプチャデータの保存に使う圧縮レベルです。",
        CheckpointInterval => "チェックポイント間隔",
        CheckpointIntervalTip => "この数のイベントごとに復元用チェックポイントを書き込みます（0 で無制限）。",
        CheckpointRatio => "チェックポイント変化率",
        CheckpointRatioTip => "キャンバスの変化がこの割合に達したらチェックポイントを書き込みます。",
        InputWatcher => "入力監視",
        InputWatcherTip => "入力の動きに合わせてキャプチャを予定します。",
        Export => "書き出し",
        Pace => "再生ペース",
        PaceTip => "1秒あたりに再生する記録済みキャプチャ数です。",
        CapturesPerSecond => "キャプチャ/秒",
        CustomPace => "カスタム再生ペース",
        Fps => "FPS",
        Quality => "品質",
        Draft => "下書き",
        Balanced => "バランス",
        High => "高品質",
        Archival => "保存用",
        Encoder => "エンコーダー",
        EncoderTip => "NVIDIA NVENC を自動検出し、利用できない場合はソフトウェア x264 を使用します。FFmpeg が必要です。",
        Auto => "自動",
        Nvenc => "NVENC",
        X264 => "x264",
        Resolution => "解像度",
        P720 => "720p",
        P1080 => "1080p",
        P1440 => "1440p",
        P2160 => "2160p",
        P75 => "75%",
        P50 => "50%",
        P25 => "25%",
        CanvasMode => "キャンバスモード",
        MaxCanvas => "最大キャンバスサイズ",
        InitialCanvas => "初期キャンバス",
        HoldEnd => "終了時に停止",
        PreviewEnd => "作品プレビューを表示",
        SecondsSuffix => " 秒",
        SkipProvisional => "仮キャプチャをスキップ",
        Duration => "動画の長さ",
        ExportVideo => "動画を書き出す",
        Exporting => "書き出しています…",
        ExportComplete => "書き出し完了",
        Planning => "書き出しを準備しています…",
        RefreshPreview => "プレビューを更新",
        UnavailableValue => "—",
        Loading => "読み込み中…",
        PreviewUnavailable => "プレビューを表示できません",
        PreviewEmpty => "キャンバスを選択するか、セッションを開いてプレビューを表示します。",
        PreviewRefreshTip => "現在のキャンバスまたはセッションの最終画像を一度読み込みます。録画中でもキャプチャは保存されず、タイムラインや統計も変わりません。",
        TooltipRefresh => "SAI プロセスとキャンバス一覧を更新します。",
        ChooseDirectory => "フォルダーを選択…",
        ChooseOutput => "動画を保存",
        AboutTitle => "timelapse4sai2 について",
        ApplicationName => super::APP_NAME,
        Version => "バージョン",
        AboutDescription => "PaintTool SAI 2 のキャンバスを録画し、タイムラプス動画として書き出します。",
        AboutLicense => "ライセンス：GNU General Public License v3.0 only（GPL-3.0-only）。",
        AboutFree => "自由でオープンソースなソフトウェアです。",
        AboutDistribution => "このソフトウェアはリンク先のリポジトリで無料で入手できます。お金を払って入手したのであれば、詐欺に遭っています。",
        FinishingBeforeExit => "終了前に処理を完了しています…",
        Mp4 => "MP4",
        Gib => "GiB",
        PointSuffix => "pt",
        PercentSuffix => "%",
    }
}

pub fn format_size(i18n: Translator, bytes: u64) -> String {
    let (value, unit) = if bytes >= 1 << 30 {
        (bytes as f64 / (1_u64 << 30) as f64, Key::Gib)
    } else if bytes >= 1 << 20 {
        (bytes as f64 / (1_u64 << 20) as f64, Key::Mib)
    } else if bytes >= 1 << 10 {
        (bytes as f64 / (1_u64 << 10) as f64, Key::Kib)
    } else {
        return format!("{} {}", bytes, i18n.tr(Key::Bytes));
    };
    format!("{value:.2} {}", i18n.tr(unit))
}

pub fn format_dimensions(width: u32, height: u32) -> String {
    format!("{} × {}", width, height)
}

pub fn format_canvas(name: &str, width: u32, height: u32) -> String {
    format!("{} — {}×{}", name, width, height)
}

pub fn format_tracked(i18n: Translator, ms: u64) -> String {
    let seconds = ms / 1000;
    match i18n.language() {
        Language::English => {
            if seconds >= 3600 {
                format!("{}h {}m", seconds / 3600, seconds / 60 % 60)
            } else if seconds >= 60 {
                format!("{}m {}s", seconds / 60, seconds % 60)
            } else {
                format!("{seconds}s")
            }
        }
        Language::SimplifiedChinese => {
            if seconds >= 3600 {
                format!("{}小时{}分", seconds / 3600, seconds / 60 % 60)
            } else if seconds >= 60 {
                format!("{}分{}秒", seconds / 60, seconds % 60)
            } else {
                format!("{seconds}秒")
            }
        }
        Language::Japanese => {
            if seconds >= 3600 {
                format!("{}時間{}分", seconds / 3600, seconds / 60 % 60)
            } else if seconds >= 60 {
                format!("{}分{}秒", seconds / 60, seconds % 60)
            } else {
                format!("{seconds}秒")
            }
        }
    }
}

pub fn format_count(count: u64) -> String {
    let digits = count.to_string();
    let mut text = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            text.push(',');
        }
        text.push(c);
    }
    text
}

/// Compact clock-style duration used for planned video length.
pub fn format_duration(ms: u64) -> String {
    let seconds = ms / 1000;
    if seconds >= 3600 {
        return format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        );
    }
    if seconds >= 60 {
        return format!("{}:{:02}", seconds / 60, seconds % 60);
    }
    let mut text = format!("0:{seconds:02}");
    if ms % 1000 != 0 {
        let fraction = format!("{:03}", ms % 1000);
        text.push('.');
        text.push_str(fraction.trim_end_matches('0'));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_has_a_value_in_every_language() {
        for &key in Key::ALL {
            for language in Language::ALL {
                assert!(
                    !Translator::new(language).tr(key).is_empty(),
                    "{language:?} is missing {key:?}"
                );
            }
        }
    }

    #[test]
    fn help_tips_are_explanations_not_repeated_labels() {
        let pairs = [
            (Key::IdleDelay, Key::IdleDelayTip),
            (Key::MinCaptureGap, Key::MinCaptureGapTip),
            (Key::MaxActiveGap, Key::MaxActiveGapTip),
            (Key::ZstdLevel, Key::ZstdLevelTip),
            (Key::CheckpointInterval, Key::CheckpointIntervalTip),
            (Key::CheckpointRatio, Key::CheckpointRatioTip),
            (Key::InputWatcher, Key::InputWatcherTip),
            (Key::Encoder, Key::EncoderTip),
        ];
        for language in Language::ALL {
            let i18n = Translator::new(language);
            for (label, tip) in pairs {
                let label = i18n.tr(label);
                let tip = i18n.tr(tip);
                assert_ne!(label, tip);
                assert!(!tip.starts_with(label), "{tip:?} repeats {label:?}");
            }
        }
    }

    #[test]
    fn tracked_time_uses_language_specific_units() {
        for (ms, en, zh, ja) in [
            (2 * 3_600_000 + 18 * 60_000, "2h 18m", "2小时18分", "2時間18分"),
            (2 * 60_000 + 4_000, "2m 4s", "2分4秒", "2分4秒"),
            (5_000, "5s", "5秒", "5秒"),
        ] {
            assert_eq!(Translator::new(Language::English).tracked(ms), en);
            assert_eq!(
                Translator::new(Language::SimplifiedChinese).tracked(ms),
                zh
            );
            assert_eq!(Translator::new(Language::Japanese).tracked(ms), ja);
        }
    }

    #[test]
    fn counts_are_grouped_consistently() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(4_821), "4,821");
        assert_eq!(format_count(1_234_567), "1,234,567");
    }

    #[test]
    fn video_duration_is_compact_without_trailing_zeroes() {
        for (ms, expected) in [
            (0, "0:00"),
            (4_000, "0:04"),
            (4_800, "0:04.8"),
            (4_205, "0:04.205"),
            (222_800, "3:42"),
            (3_600_000, "1:00:00"),
        ] {
            assert_eq!(format_duration(ms), expected);
        }
    }

    #[test]
    fn language_names_stay_native() {
        for language in Language::ALL {
            let i18n = Translator::new(language);
            assert_eq!(i18n.tr(Key::English), "English");
            assert_eq!(i18n.tr(Key::Japanese), "日本語");
            assert_eq!(i18n.tr(Key::SimplifiedChinese), "简体中文");
        }
    }
}
