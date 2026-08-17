# Cortex 中文版改动说明

本仓库是 [PndaMan/cortex](https://github.com/PndaMan/cortex) 的 Apache-2.0 派生版本。

## 1.0.35-zh.1

- 新增简体中文界面，支持切换回英语并保存偏好。
- 新生成的笔记、题目、闪卡、考试、总结和聊天回答默认使用简体中文。
- 新增 OpenAI 兼容的自定义 Embedding 端点，适配阿里云百炼等服务。
- 自定义生成与 Embedding 端点均支持 `http://` 和 `https://`，可连接 localhost 或局域网服务。
- 新增全局思考力度选择器；自动适配 DeepSeek V4、OpenRouter 和 OpenAI 推理参数。
- 使用独立 Bundle ID `study.cortex.zh`，可与官方 Cortex 并存。
- 移除了上游自动更新配置，避免官方版本覆盖此派生版本。

原项目的版权、署名和 `LICENSE` 均原样保留；本派生项目与上游没有官方隶属关系。
