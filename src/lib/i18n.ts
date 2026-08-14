/**
 * Lightweight runtime localisation for the standalone Chinese edition.
 *
 * Cortex upstream has no i18n layer and its UI strings are spread across many
 * Svelte components. Translating rendered, known UI strings keeps this edition
 * maintainable while deliberately leaving user notes, source text, model output,
 * code, and editable fields untouched.
 */
export type UiLocale = "zh-CN" | "en";

const DEFAULT_LOCALE: UiLocale = "zh-CN";
const CACHE_KEY = "cortex-ui-language";
let locale: UiLocale = DEFAULT_LOCALE;
let observer: MutationObserver | null = null;

type RenderedValue = { source: string; rendered: string };
const textState = new WeakMap<Text, RenderedValue>();
const attrState = new WeakMap<Element, Map<string, RenderedValue>>();

// Exact interface labels. Model names, URLs, and user content are intentionally
// absent: they are data, not UI copy, and should remain exactly as entered.
const zh: Record<string, string> = {
  "Home": "首页",
  "Dashboard": "概览",
  "Settings": "设置",
  "Subjects": "课程",
  "Subject": "课程",
  "Sources": "资料",
  "Source": "资料",
  "Notes": "笔记",
  "Note": "笔记",
  "Calendar": "日历",
  "Analytics": "学习分析",
  "Insights": "学习洞察",
  "Exams": "考试",
  "Exam": "考试",
  "Materials": "学习材料",
  "Chats": "问答",
  "Chat": "问答",
  "Search": "搜索",
  "Global search": "全局搜索",
  "Find": "查找",
  "Help": "帮助",
  "Back": "返回",
  "Next": "下一步",
  "Previous": "上一步",
  "Close": "关闭",
  "Cancel": "取消",
  "Confirm": "确认",
  "Done": "完成",
  "Save": "保存",
  "Save changes": "保存更改",
  "Save settings": "保存设置",
  "Save profile": "保存个人资料",
  "Save keys": "保存密钥",
  "Saved": "已保存",
  "Delete": "删除",
  "Edit": "编辑",
  "Rename": "重命名",
  "Add": "添加",
  "Create": "创建",
  "Clear": "清除",
  "Retry": "重试",
  "Refresh": "刷新",
  "Copy": "复制",
  "Copied": "已复制",
  "Download": "下载",
  "Upload": "上传",
  "Import": "导入",
  "Export": "导出",
  "Open": "打开",
  "Open settings": "打开设置",
  "Loading…": "正在加载…",
  "Loading...": "正在加载…",
  "Generating…": "正在生成…",
  "Generating...": "正在生成…",
  "Saving…": "正在保存…",
  "Saving...": "正在保存…",
  "Checking…": "正在检查…",
  "Checking...": "正在检查…",
  "Verifying…": "正在验证…",
  "Verifying...": "正在验证…",
  "Connected": "已连接",
  "connected": "已连接",
  "Not set": "未设置",
  "not set": "未设置",
  "Not checked": "未检查",
  "not checked": "未检查",
  "saved": "已保存",
  "Hide": "隐藏",
  "Show": "显示",
  "Notifications": "通知",
  "Minimize sidebar": "收起侧栏",
  "Record lecture": "录制课程",
  "New subject": "新建课程",
  "Lofi Girl lo-fi · ad-free study": "Lofi Girl 轻音乐 · 无广告学习",
  "Study sound": "学习音乐",
  "Models": "模型",
  "A model for every task": "为每项任务选择模型",
  "Route each job to the provider that does it best. Token budgets cap spend per call.": "为每项任务选择最合适的服务商；令牌预算用于限制每次调用的成本。",
  "Task": "任务",
  "Provider": "服务商",
  "Model": "模型",
  "Token budget": "令牌预算",
  "Thinking effort": "思考力度",
  "Provider default": "服务商默认",
  "Off": "关闭思考",
  "Low": "低",
  "Medium": "中",
  "High": "高",
  "Maximum": "最高",
  "Controls only providers with a supported reasoning API. DeepSeek V4 keeps low, maps medium and high to high, and maps maximum to max; OpenRouter forwards the selected effort.": "仅对支持推理参数的服务商生效。DeepSeek V4 的低保持为低，中、高映射为高，最高映射为 max；OpenRouter 会转发所选力度。",
  "Scoped Q&A across sources": "在选定资料范围内问答",
  "Cheatsheet synthesis": "知识速览生成",
  "Completeness-checked merges": "检查完整性的合并整理",
  "Audio overview script": "音频概览脚本",
  "Two-host podcast dialogue": "双主持人播客对话",
  "Quiz generation": "题目生成",
  "MCQ · short answer · cloze": "选择题 · 简答题 · 完形填空",
  "Flashcard generation": "闪卡生成",
  "Q/A pairs + SRS scheduling": "问答卡片 + 间隔重复安排",
  "Embedding": "向量模型",
  "Vector index for retrieval": "用于检索的向量索引",
  "Custom endpoint": "自定义端点",
  "Ollama (local)": "Ollama（本地）",
  "Ollama tasks run fully offline on this machine or your homelab — no key required.": "Ollama 任务完全在本机或你的家庭服务器离线运行，无需 API 密钥。",
  "API keys": "API 密钥",
  "Bring your own keys": "使用你自己的密钥",
  "Stored in the OS keychain, never synced. Nothing routes through Cortex servers.": "密钥保存在系统钥匙串中，不会同步；请求不会经过 Cortex 服务器。",
  "Providers": "服务商",
  "Custom endpoint URL": "自定义端点 URL",
  "OpenAI-compatible base URL": "OpenAI 兼容的基础 URL",
  "OpenAI-compatible base URL — Bailian example: https://{WorkspaceId}.cn-beijing.maas.aliyuncs.com/compatible-mode/v1": "OpenAI 兼容的基础 URL；百炼示例：https://{WorkspaceId}.cn-beijing.maas.aliyuncs.com/compatible-mode/v1",
  "OpenAI-compatible HTTP(S) base URL. HTTP is allowed for localhost and LAN.": "OpenAI 兼容的 HTTP(S) 基础 URL；支持 localhost 和局域网使用 HTTP。",
  "OpenAI-compatible HTTP(S) base URL. HTTP is allowed for localhost and LAN. Bailian example: https://{WorkspaceId}.cn-beijing.maas.aliyuncs.com/compatible-mode/v1": "OpenAI 兼容的 HTTP(S) 基础 URL；支持 localhost 和局域网使用 HTTP。百炼示例：https://{WorkspaceId}.cn-beijing.maas.aliyuncs.com/compatible-mode/v1",
  "http://localhost:8000/v1 or https://…/v1": "http://localhost:8000/v1 或 https://…/v1",
  "Custom endpoint API key": "自定义端点 API 密钥",
  "Bearer token for the custom endpoint": "自定义端点的 Bearer 令牌",
  "Custom embedding endpoint URL": "自定义向量端点 URL",
  "Custom embedding API key": "自定义向量 API 密钥",
  "Used only for custom Embedding; kept separate from custom chat": "仅用于自定义向量模型，和自定义问答端点分开保存。",
  "Custom Embedding uses its own endpoint and API key in API keys. It supports OpenAI-compatible providers such as Bailian: choose": "自定义向量模型使用独立的端点和 API 密钥。它支持百炼等 OpenAI 兼容服务：选择",
  ", save the endpoint and key, then test it.": "，保存端点和密钥后再进行测试。",
  "Test embedding": "测试向量模型",
  "Testing…": "正在测试…",
  "Testing...": "正在测试…",
  "Embedding connected": "向量模型已连接",
  "Embedding test failed": "向量模型测试失败",
  "Verify": "验证",
  "Local models (Ollama)": "本地模型（Ollama）",
  "Ollama URL": "Ollama URL",
  "UNREACHABLE": "无法连接",
  "leave blank to use your Homelab.": "留空则使用你的家庭服务器。",
  "Keyless, local. Defaults to": "无需密钥，本地运行。默认地址：",
  "Appearance": "外观",
  "Make it yours": "打造你的学习空间",
  "Cortex re-skins live from your Omarchy theme, or pick one manually.": "Cortex 可跟随 Omarchy 主题实时换肤，也可以手动选择主题。",
  "Language": "语言",
  "Interface and AI output language": "界面与 AI 输出语言",
  "Simplified Chinese": "简体中文",
  "English": "英语",
  "Theme": "主题",
  "Follow Omarchy theme": "跟随 Omarchy 主题",
  "Manual theme": "手动选择主题",
  "Reading": "阅读",
  "Cheatsheet typeface": "知识速览字体",
  "The voice of everything you read to learn.": "学习内容的阅读字体。",
  "Mono": "等宽",
  "Serif": "衬线",
  "Sans": "无衬线",
  "Density": "界面密度",
  "Regular": "常规",
  "Compact": "紧凑",
  "Data & privacy": "数据与隐私",
  "Local-first by default": "默认本地优先",
  "Everything lives in a SQLite database on this machine. You own it.": "所有内容都保存在本机的 SQLite 数据库中，由你掌控。",
  "Storage": "存储",
  "Database": "数据库",
  "Vector index": "向量索引",
  "Local embeddings for retrieval": "用于检索的本地向量索引",
  "Offline mode": "离线模式",
  "Block all network calls; Ollama only.": "阻止所有网络请求；仅允许 Ollama。",
  "Profile": "个人资料",
  "Who the AI thinks you are": "让 AI 了解你",
  "Shared with every chat and generation so answers fit your level and style. Stays on this machine.": "会用于每次问答与生成，让回答贴合你的水平和风格；信息仅保留在本机。",
  "IDENTITY": "身份信息",
  "Display name": "显示名称",
  "Pronouns": "代词",
  "Level": "学习阶段",
  "Field of study": "学习领域",
  "About you": "关于你",
  "Context the AI uses to personalize explanations.": "AI 用它来个性化解释的背景信息。",
  "In your words": "用你的话描述",
  "Response style": "回答风格",
  "How much detail by default.": "默认回答的详细程度。",
  "Explain with": "解释方式",
  "Pick what helps you learn fastest.": "选择最能帮助你学习的方式。",
  "worked examples": "示例推导",
  "analogies": "类比说明",
  "formal proofs": "形式化证明",
  "diagrams": "图示",
  "code snippets": "代码片段",
  "Concise": "简洁",
  "Balanced": "平衡",
  "Detailed": "详细",
  "MEMORY": "长期记忆",
  "Long-term facts the AI is given in every chat — like remembering your exam date, the textbook you use, or how you like answers framed.": "每次问答都会提供给 AI 的长期信息，例如考试日期、使用的教材和你偏好的回答方式。",
  "e.g. My final exam is on June 20th": "例如：我的期末考试在 6 月 20 日",
  "Remember": "记住",
  "No memories yet. Add a fact above and the AI will keep it in mind.": "暂时没有长期记忆。添加一条信息后，AI 会在后续问答中记住它。",
  "Learning style": "学习偏好",
  "What the AI receives": "AI 会收到的内容",
  "Keys saved": "密钥已保存",
  "Stored in the system keychain.": "已保存到系统钥匙串。",
  "Profile saved": "个人资料已保存",
  "The AI will use your updated context.": "AI 将使用更新后的个人背景信息。",
  "Save failed": "保存失败",
  "Update failed": "更新失败",
  "Add failed": "添加失败",
  "Delete failed": "删除失败",
  "Failed to load": "加载失败",
  "Error": "错误",
  "Warning": "警告",
  "Success": "成功",
  "Info": "提示",
  "Generate": "生成",
  "Generate quiz": "生成题目",
  "Generate flashcards": "生成闪卡",
  "Generate material": "生成学习材料",
  "Start studying": "开始学习",
  "Add source": "添加资料",
  "Add subject": "添加课程",
  "Upload a file": "上传文件",
  "Drop files here": "将文件拖到这里",
  "Choose files": "选择文件",
  "Paste URL": "粘贴 URL",
  "No sources yet": "还没有资料",
  "No subjects yet": "还没有课程",
  "Add your first subject to start building cheatsheets, flashcards and more.": "添加第一门课程，即可开始生成知识速览、闪卡等学习材料。",
  "No notes yet": "还没有笔记",
  "Create your first subject to get started.": "创建第一门课程，开始学习。",
  "Create a subject": "创建课程",
  "Name": "名称",
  "Title": "标题",
  "Description": "说明",
  "Tags": "标签",
  "Topic": "主题",
  "Topics": "主题",
  "All topics": "全部主题",
  "All sources": "全部资料",
  "Recent": "最近使用",
  "Today": "今天",
  "Yesterday": "昨天",
  "Tomorrow": "明天",
  "This week": "本周",
  "Study": "学习",
  "Review": "复习",
  "Due": "到期",
  "Start": "开始",
  "Pause": "暂停",
  "Resume": "继续",
  "Stop": "停止",
  "Complete": "完成",
  "Completed": "已完成",
  "In progress": "进行中",
  "Not started": "未开始",
  "Question": "问题",
  "Answer": "答案",
  "Correct": "正确",
  "Incorrect": "错误",
  "Score": "得分",
  "Submit": "提交",
  "Check answers": "检查答案",
  "New chat": "新建对话",
  "Clear chat": "清空对话",
  "Ask anything about your sources…": "就你的资料提出任何问题…",
  "Search everything…": "搜索全部内容…",
  "Search sources…": "搜索资料…",
  "Search notes…": "搜索笔记…",
  "No results": "没有结果",
  "No results found": "未找到结果",
  "Settings saved": "设置已保存",
  "Language changed": "语言已切换",
  "UI and new AI-generated content now follow this language.": "界面和新生成的 AI 内容将使用此语言。",
  "Version": "版本",
  "Updates": "更新",
  "Custom Chinese edition": "Cortex 中文版",
  "This edition does not install upstream Cortex updates automatically, so your Chinese interface and separate local data stay intact.": "此版本不会自动安装上游 Cortex 更新，以保留中文界面和独立的本地数据。",
  "Check for updates": "检查更新",
  "You're up to date": "已是最新版本",
  "Downloading update…": "正在下载更新…",
  "Update & restart": "更新并重启",
  "Update check failed": "检查更新失败",
  "Update failed to install": "更新安装失败",
  "About": "关于",
  "Keyboard shortcuts": "键盘快捷键",
  "Integrations": "集成",
  "Experimental": "实验功能",
  "Audio": "音频",
  "Google Calendar": "Google 日历",
  "Keybinds": "快捷键",
  "Privacy": "隐私",
  "Back to focus": "回到专注",
  "Break time": "休息时间",
  "Long break": "长休息",
  "Lecture summary ready": "课程摘要已准备好",
  "Key points + terms saved to Notes.": "要点和术语已保存到笔记。",
  "Subject updated": "课程已更新",
  "Source updated": "资料已更新",
  "Topic deleted": "主题已删除",
  "Sources recovered": "资料已恢复",
  "Retrying failed sources": "正在重试失败的资料",
  "Sync failed": "同步失败",
  "Subject not found": "未找到课程",
};

const patterns: Array<[RegExp, (...parts: string[]) => string]> = [
  [/^(\d+) chunks$/, (n) => `${n} 个片段`],
  [/^(\d+) sources$/, (n) => `${n} 份资料`],
  [/^(\d+) source$/, (n) => `${n} 份资料`],
  [/^(\d+) questions$/, (n) => `${n} 道题目`],
  [/^(\d+) cards$/, (n) => `${n} 张闪卡`],
  [/^(\d+) slides$/, (n) => `${n} 张幻灯片`],
  [/^(\d+) minutes?$/, (n) => `${n} 分钟`],
  [/^Session (\d+) — let's go\.$/, (n) => `第 ${n} 轮，开始专注。`],
  [/^(\d+) re-ingested successfully\.$/, (n) => `已成功重新处理 ${n} 份资料。`],
];

function translateValue(source: string): string {
  const match = source.match(/^(\s*)([\s\S]*?)(\s*)$/);
  if (!match) return source;
  const [, before, value, after] = match;
  if (!value || value.includes("\n")) return source;
  let translated = zh[value];
  if (!translated) {
    for (const [pattern, make] of patterns) {
      const parts = value.match(pattern);
      if (parts) {
        translated = make(...parts.slice(1));
        break;
      }
    }
  }
  return translated ? `${before}${translated}${after}` : source;
}

function excluded(element: Element | null): boolean {
  if (!element) return true;
  return !!element.closest(
    "script, style, noscript, textarea, input, pre, code, [contenteditable='true'], [data-i18n-skip], .markdown, .md, .rich-editor, .source-content"
  );
}

function renderText(text: Text) {
  if (excluded(text.parentElement)) return;
  const current = text.data;
  const previous = textState.get(text);
  const source = previous && (current === previous.source || current === previous.rendered)
    ? previous.source
    : current;
  const rendered = locale === "zh-CN" ? translateValue(source) : source;
  textState.set(text, { source, rendered });
  if (current !== rendered) text.data = rendered;
}

const translatableAttributes = ["title", "placeholder", "aria-label", "alt"];

function renderAttribute(element: Element, attribute: string) {
  if (excluded(element) || !element.hasAttribute(attribute)) return;
  const current = element.getAttribute(attribute) ?? "";
  let states = attrState.get(element);
  if (!states) {
    states = new Map();
    attrState.set(element, states);
  }
  const previous = states.get(attribute);
  const source = previous && (current === previous.source || current === previous.rendered)
    ? previous.source
    : current;
  const rendered = locale === "zh-CN" ? translateValue(source) : source;
  states.set(attribute, { source, rendered });
  if (current !== rendered) element.setAttribute(attribute, rendered);
}

function renderTree(root: Node) {
  if (root.nodeType === Node.TEXT_NODE) {
    renderText(root as Text);
    return;
  }
  if (root.nodeType !== Node.ELEMENT_NODE && root.nodeType !== Node.DOCUMENT_FRAGMENT_NODE) return;
  const element = root.nodeType === Node.ELEMENT_NODE ? root as Element : null;
  if (element) {
    for (const attribute of translatableAttributes) renderAttribute(element, attribute);
  }
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  let node: Node | null;
  while ((node = walker.nextNode())) renderText(node as Text);
  if (root.nodeType === Node.ELEMENT_NODE) {
    for (const child of (root as Element).querySelectorAll("[title], [placeholder], [aria-label], [alt]")) {
      for (const attribute of translatableAttributes) renderAttribute(child, attribute);
    }
  }
}

function translateDocument() {
  if (document.body) renderTree(document.body);
}

function observe() {
  if (observer || !document.documentElement) return;
  observer = new MutationObserver((records) => {
    for (const record of records) {
      if (record.type === "characterData") {
        renderText(record.target as Text);
      } else if (record.type === "attributes") {
        renderAttribute(record.target as Element, record.attributeName ?? "");
      } else {
        for (const added of record.addedNodes) renderTree(added);
      }
    }
  });
  observer.observe(document.documentElement, {
    subtree: true,
    childList: true,
    characterData: true,
    attributes: true,
    attributeFilter: translatableAttributes,
  });
}

export function getUiLocale(): UiLocale {
  return locale;
}

export function setUiLocale(next: UiLocale, cache = true) {
  locale = next === "en" ? "en" : "zh-CN";
  document.documentElement.lang = locale;
  document.documentElement.setAttribute("data-ui-language", locale);
  if (cache) {
    try { localStorage.setItem(CACHE_KEY, locale); } catch { /* storage unavailable */ }
  }
  translateDocument();
}

export function initializeUiLocale() {
  let cached: UiLocale | null = null;
  try {
    const value = localStorage.getItem(CACHE_KEY);
    if (value === "en" || value === "zh-CN") cached = value;
  } catch { /* storage unavailable */ }
  setUiLocale(cached ?? DEFAULT_LOCALE, false);
  observe();
}
