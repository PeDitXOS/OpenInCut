# OpenInCut

跨平台视频编辑器，专为内容创作者设计，具备 AI 超能力：**文本编辑**（Whisper 逐字级）、**一键消除静音**、**自动竖屏视频**、**情感反应虚拟形象**、**卡拉OK 风格字幕**、**文本转语音**，以及 **内置 MCP 服务器**，让 AI 智能体能为您编辑项目。

[English](README.md) | [فارسی](README-fa.md) | [العربية](README-ar.md) | [Русский](README-ru.md) | [中文](README-zh.md)

---

## ✨ 功能特性

### 基于文本的编辑
使用 Whisper 转录视频（逐字级时间戳），然后通过删除转录文本中的词语来编辑 — 视频会自动剪切。

### AI 驱动的编辑
内置拥有 53 个工具的 MCP 服务器。连接 Claude、ChatGPT 或任何 LLM，通过自然语言编辑您的项目：
- "添加一个写着欢迎的标题"
- "移除 0:15 到 0:20 的静音"
- "修正这个片段的透视"
- "添加中文字幕"

### 调色与特效
- **调色**：亮度、对比度、饱和度、伽马
- **Chroma Key**：绿幕/蓝幕抠像
- **高斯模糊**：背景模糊
- **投影阴影**：叠加层阴影效果
- **透视校正**：地平线校正（AI 辅助）
- **垂直填充**：竖屏视频的模糊背景

### 字幕与 TTS
- 从转录自动生成字幕（逐字或卡拉 OK 模式）
- 中文语言支持
- RTL 文本方向支持从右向左的语言
- 使用 Kokoro-82M 的文本转语音（54 个声音，8 种语言）

### 时间轴编辑
- 拖拽式时间轴，带磁性吸附
- 多轨道合成
- 所有属性的关键帧动画
- 11 种转场类型（淡入淡出、擦除、滑动、溶解等）
- 速度控制（0.25x–4x）并保持音调

### 导出
- MP4、WebM、GIF 格式
- 可自定义分辨率和质量
- 支持片段范围的批量导出

---

## 🚀 快速开始

### 系统要求
- FFmpeg 6+ 在 PATH 中
- Rust (stable) + Node 20+

### 安装
```bash
git clone https://github.com/PeDitXOS/OpenInCut.git
cd OpenInCut
npm install
npx tauri dev
```

或从 [GitHub Releases](https://github.com/PeDitXOS/OpenInCut/releases) 下载预构建版本。

---

## 🔌 MCP 服务器

OpenInCut 启动时在 `http://127.0.0.1:4599/mcp` 运行 MCP 服务器。任何会 MCP 的 AI 智能体都能编辑您的项目：

```bash
claude mcp add --transport http opencut http://127.0.0.1:4599/mcp \
  --header "Authorization: Bearer YOUR_TOKEN"
```

53 个工具可用 — 查看 [docs/MCP.md](docs/MCP.md)。

---

## 🌐 语言支持

| 功能 | 支持的语言 |
|---------|-------------------|
| Whisper 转录 | 英语、中文、阿拉伯语、西班牙语、葡萄牙语、法语、德语 |
| TTS 语音 | 英语 (US/GB)、西班牙语、法语、意大利语、日语、印地语、葡萄牙语、普通话 |
| 文本方向 | LTR (英语等) 和 RTL (阿拉伯语、波斯语、希伯来语) |
| UI | 英语 (本地化就绪) |

---

## 🏗️ 架构

九个 crate（纯模型、媒体、音频、渲染、文本、导出、AI、whisper、Tauri shell），预览和导出共享一个合成器，全程微秒级精度。

📖 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)

---

## 致谢

OpenInCut 基于 **HectorPulido** 的 [UberEditor](https://github.com/HectorPulido/UberEditor)。感谢在 MCP 服务器架构、时间轴引擎和 AI 集成框架方面的原创工作。

**许可证**：Apache-2.0（与原版相同）。版权所有 2026 PeDitXOS 及贡献者。

---

## 贡献

欢迎贡献！请参阅 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 许可证

Apache-2.0 — 详见 [LICENSE](LICENSE)。