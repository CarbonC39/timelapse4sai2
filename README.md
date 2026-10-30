# timelapse4sai2

Linux 原生 Rust CLI，专用于 **SAI Ver.2 64bit – 2026-07-02b Technical Preview**。
固定 session RVA `0x32eb20`，可执行文件 MD5 必须为 `6f3f351303cf3896f4c2977925c994c2`。

```sh
cargo build --release
./target/release/timelapse4sai2 detect
./target/release/timelapse4sai2 canvases
./target/release/timelapse4sai2 dump 0 test.png
# 多进程时显式指定 PID（选项放在子命令之前）
./target/release/timelapse4sai2 --pid 1234 dump 0 test.png
cargo install --path . --locked
```

NixOS 如未配置 Rust 工具链，可使用 `nix-shell -p cargo rustc` 后构建。
不依赖 Windows DLL、OpenCV、显示服务器或屏幕截图；宿主通过 `process_vm_readv` 直接读取 Wine 进程。
Bottles/Flatpak 的可执行文件通过 `/proc/PID/root` 打开并校验 MD5。
模块基址为 offset-zero、包含有效 x64 PE32+ 头的 `sai2.exe` 映射起点，不对 PE section 的文件偏移机械相减。

## 读取布局

- `*(base + RVA)` 是链表哨兵，读取其 `next_canvas` 后开始枚举。
- `*(canvas + 0x28)` 为 level 0 slot，再读取 `*slot` 获得 tile map。
- `tree[y]` 是行指针，`row[x]` 是 256×256×4 tile。
- 校验 map 尺寸与 canvas 一致、tile 数量精确等于向上取整的尺寸。
- 直接复制有效像素至 RGB buffer，避免分配完整 padding buffer。

布局依据：[原项目 sai.py](https://github.com/cromachina/art-timelapse/blob/main/src/art_timelapse/sai.py)。
原项目 [Dev notes](https://github.com/cromachina/art-timelapse#dev-notes) 描述内存通道为 BGRA；
OpenCV 接受 BGR，而 PNG 接受 RGB，因此默认交换红蓝通道并丢弃第四通道。
如现场验证目标内存为 RGBA，可显式使用 `dump 0 test.png --channel-order rgba`。
不会凭像素值猜测通道顺序。

## 限制与现场验收

工具拒绝覆盖输出文件；读取失败不生成 PNG。画布尺寸上限为每边 100000，RGB 分配上限 1 GiB。
链表循环、无效指针、UTF-16 错误、tile 缺失均报错，不用黑色像素掩盖读取失败。
读取结束会复核元数据和指针，但不会暂停 SAI；绘制期间同一 tile 的原地更新无法保证原子快照。
请在画布静止时导出，关闭或切换画布后重新执行 `canvases`，索引不是持久标识。

若出现 EPERM/EACCES，检查同一用户、`/proc/sys/kernel/yama/ptrace_scope` 和容器的 ptrace 权限。
Yama 为 1 时同一用户也未必允许读取同级进程；需由本机策略允许的调试权限或启动父子关系解决。
工具不会自动更改系统安全设置，也不会退回 `/proc/PID/mem`。

实际验收需运行该版本 SAI：创建含纯红、绿、蓝及边缘标记的非 256 整倍数画布，
用 SAI 导出无透明 RGB PNG 作基准，对比解码后的像素和尺寸（PNG 文件字节不一定一致）。
缩放、旋转和遮挡窗口后重复导出，对比像素应完全一致；再覆盖多画布和空画布场景。
合成内存测试仅验证布局、裁切和通道转换，不能代替真实进程验收。

```sh
cargo test --locked
```

## 增量录制 v1

```sh
cargo build --release --locked
./target/release/timelapse4sai2 record 0 ./recordings/test
# 禁用输入监听，仅按 max-active-gap 定时捕获：
./target/release/timelapse4sai2 record 0 ./recordings/test --no-input-watch
```

Ctrl-C / SIGTERM 停止调度，补一次 Stable candidate，排空队列并输出统计。
既有目录会校验、恢复并继续追加，同一目录仅允许一个 writer。
参数默认值统一在 Config 中，`--help` 可查看：idle-delay 150ms、min-capture-gap 300ms、
max-active-gap 3000ms、zstd-level 1、checkpoint-every-events 500、checkpoint-changed-ratio 70。
两个 checkpoint 参数各设为 0 禁用对应触发器；初始状态与 resize 始终保存 checkpoint。
时间间隔按单调时钟独立计算，最小冷却优先；Unix timestamp 仅作 timeline 元数据。
无输入监听时定时产生 Provisional candidate，停止时补 Stable candidate。

监听 Linux `/dev/input/event*` 键盘、鼠标、数位板活动，不过滤窗口。
需宿主 ACL/组授予当前用户读取权限；不自动提权或抓取设备。每 5 秒扫描新设备。
没有可用设备时明确报错，可显式使用 `--no-input-watch`。

只有 composite 可见像素变化才写 logical frame。热路径直接读取 raw tiles，
不分配完整 RGB framebuffer；第四通道与边界 padding 归零，保存规范化 BGRX。
这相对于原 dump 的可见 RGB 无损。XXH3-128 用作 tile hash 与全局 blob ID，
当前依赖其极低碰撞率，verify_identity 为未来 byte-equality 检查留出扩展位置。
Undo 回退复用历史 blob，仅新 blob 进行单线程 zstd 压缩。

主线程作为 capture worker，唯一额外线程负责压缩与存储。两个 tile message 的有界队列
对捕获施加背压，不暂停 SAI，不丢弃已排队 Stable candidate；tile buffer 可回收复用。
只保留当前/candidate 索引、全局 blob 索引及少量 tile，不缓存历史图像。
blob 索引内存随 unique blobs 增长；存储慢会拉长候选间隔，不能记录每一瞬时状态。
结构校验失败会 Abort，未引用的已写 blob 可保留；失败候选有重试机会，连续 10 次失败
或画布关闭则明确停止。SAI 原地像素修改仍不保证原子快照。

活跃选择使用地址 + SAI id，manifest 保存 name/path，不将 canvas index 当作持久身份。
这些不是跨进程文档 UUID；继续 session 需明确选择且名字/路径一致，
无法证明同名文件或地址/id 重用一定是原文档，不自动跨画布追踪。
二进制布局与恢复保证见 [存储格式](docs/recording-format.md)。

停止后可独立校验，不需要运行 SAI：

```sh
./target/release/timelapse4sai2 verify ./recordings/live-test
./target/release/timelapse4sai2 verify ./recordings/live-test --dump-final recovered.png
```

`verify` 会验证 blob 与引用并恢复状态，也会截去不完整尾部；活跃 writer 持锁时拒绝打开。
`--dump-final` 离线合成最终 RGB PNG，不影响 recorder 热路径；已有 PNG 不覆盖。
默认录制产物目录 `/recordings/` 已加入 gitignore。

## 简单视频预览

需要 PATH 中有支持 libx264 的 `ffmpeg`：

```sh
./target/release/timelapse4sai2 export ./recordings/live-test ./recordings/preview.mp4
# 放慢，每个 logical state 显示 1 秒：
./target/release/timelapse4sai2 export ./recordings/live-test ./recordings/slow.mp4 --fps 1 --size 1280
```

默认每秒 2 个状态，包含 Stable、Provisional 和 resize 状态，忽略 wall-clock 间隔。
checkpoint 不额外产生视频帧。画面等比缩放并居中放入 1280×1280 灰底，最近邻缩放用于粗略检查；
输出为 H.264 MP4 / yuv420p / 30fps（重复显示状态以保持时长）。
`--fps` 范围 1..60，`--size` 为 64..4096 的偶数；视频输出不覆盖已有文件。
仅离线导出时合成 RGB，按顺序处理当前状态，不缓存历史帧。尚不支持结构 Transform 事件导出。
