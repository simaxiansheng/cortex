# Cortex 中文版

这是 [PndaMan/cortex](https://github.com/PndaMan/cortex) 的本地化派生版本，面向希望在本机把 PDF、PPT、DOCX、网页和课程录音转为可复习材料的中文用户。

> 上游项目采用 Apache-2.0 许可证；本仓库保留原始 `LICENSE`、版权和署名信息。本版本与上游项目没有官方隶属关系。

## 本版本改动

- 默认简体中文界面，可在“设置 → 外观 → 语言”随时切回英语；选择语言同时决定新的 AI 生成内容语言。
- 题目、闪卡、学习笔记、考试和聊天回答默认要求使用简体中文；不会翻译你上传的原始资料。
- Embedding 可独立选择 Gemini、OpenAI、Ollama 或任意 OpenAI 兼容端点。
- 支持阿里云百炼（Model Studio）等自定义向量服务；生成模型与向量模型可使用不同服务商。
- 使用独立 Bundle ID 和数据目录，因此可与官方 `Cortex.app` 并存，互不覆盖。
- 已禁用上游自动更新，避免官方英文版本覆盖本地化版本。

## 快速配置

1. 在“设置 → API 密钥”填入你使用的生成模型密钥，例如 OpenRouter / DeepSeek。
2. 在“设置 → 模型”给聊天、题目、闪卡等任务选择模型。
3. 为向量检索选择 `Custom endpoint`，在“API 密钥”填写“自定义向量端点 URL”和“自定义向量 API 密钥”。
4. 在向量模型名称中填写服务商模型 ID，然后点击“测试向量模型”。

自定义端点允许使用 `http://` 或 `https://`。因此本机服务可填 `http://localhost:8000/v1`，局域网服务可填类似 `http://192.168.1.42:8000/v1`；请只连接你信任的局域网服务。

### 思考力度

在“设置 → 模型”中选择全局“思考力度”。默认值不会覆盖服务商设置；选择后会自动按服务商协议发送参数：直连 DeepSeek V4 启用/关闭 `thinking` 并使用 `reasoning_effort`，OpenRouter 使用 `reasoning.effort`。DeepSeek V4 的 `low`、`medium`、`high` 均会映射为 `high`，`maximum` 映射为 `max`。

### 百炼 Embedding 示例

- 服务商：`Custom endpoint`
- 模型：`text-embedding-v4`
- 基础 URL（北京地域）：
  `https://{WorkspaceId}.cn-beijing.maas.aliyuncs.com/compatible-mode/v1`
- API 密钥：你的百炼 API Key

本版本调用的是 OpenAI 兼容的 `/embeddings` 接口。[百炼官方文档](https://help.aliyun.com/zh/model-studio/embedding-interfaces-compatible-with-openai)说明该接口支持 `text-embedding-v4`，并要求将 `{WorkspaceId}` 替换为实际业务空间 ID。

## 数据与隐私

应用数据库与向量索引默认保留在本机。只要你使用云端模型，发送给模型服务商的文本片段就会离开设备；请不要上传不应交给该服务商处理的机密资料。

## macOS 安装

从 GitHub Releases 下载 `Cortex 中文版` 的 DMG，拖入“应用程序”目录。这个个人构建版不带 Apple 公证签名，首次打开时 macOS 可能要求你在“隐私与安全性”中确认打开。

## 从源码构建

```bash
bun install --frozen-lockfile
bun run check
bun run tauri build -- --config src-tauri/tauri.macos.conf.json
```

需要 Rust stable、Bun 和 macOS Command Line Tools。构建产物位于 `src-tauri/target/release/bundle/`。

## 上游与许可证

- 上游：<https://github.com/PndaMan/cortex>
- 许可证：[Apache License 2.0](LICENSE)
