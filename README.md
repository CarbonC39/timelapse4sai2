# sai-memory-probe

第一阶段 Linux CLI，专用于 **SAI Ver.2 64bit – 2026-07-02b Technical Preview**。
固定 session RVA `0x32eb20`，可执行文件 MD5 必须为 `6f3f351303cf3896f4c2977925c994c2`。

```sh
cargo build --release
./target/release/sai-memory-probe detect
./target/release/sai-memory-probe canvases
./target/release/sai-memory-probe dump 0 test.png
# 多进程时显式指定 PID（选项放在子命令之前）
./target/release/sai-memory-probe --pid 1234 dump 0 test.png
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

## 本次实机验证

已在 Bottles/Wine 中通过目标版本 MD5 校验，定位 PE 基址 `0x140000000`，
并成功使用宿主 `process_vm_readv` 导出：

- 单画布 `gcc.sai2`（2000×2500）：用户确认导出与 SAI 中一致。
- 双画布 `gcc.sai2`（2000×2500）和 `preview.sai2`（200×200）：枚举和分别导出成功，
  原画布 PNG 与单画布时字节完全一致，新增画布 PNG 尺寸及解码正常。

实际验证发现 Bottles 自身也会映射 `sai2.exe` 文件，因此检测同时要求进程名称或命令行匹配，
并正确处理 Wine 的 Windows 路径及 `comm` 末尾换行。对应回归测试已加入。
缩放、旋转与遮挡后的逐像素对照尚未单独实测。
