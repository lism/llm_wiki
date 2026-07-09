# LLM Wiki 独立工具使用说明

本目录包含两个从桌面应用中抽取出来的独立命令行工具。

---

## 一、llm-wiki-api — 独立 API 服务器

### 简介

将桌面应用内置的本地 HTTP API（`127.0.0.1:19828`）抽取为独立进程，**不依赖 Tauri 窗口、WebView 或系统托盘**。适合部署在服务器或 NAS 上供外部工具调用。

### 编译

```bash
cargo build --release --bin llm-wiki-api
```

产物位于 `target/release/llm-wiki-api`。

### 运行

```bash
./llm-wiki-api --data-dir ~/Library/Application\ Support/com.llm-wiki.app
```

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `--data-dir` | 平台默认应用数据目录 | 包含 `app-state.json` 的配置目录 |
| `--port` | 19828 | 监听端口 |
| `--host` | `127.0.0.1` | 绑定地址（也可通过 `LLM_WIKI_BIND_HOST` 环境变量设置） |

### 环境变量

| 变量 | 说明 |
|------|------|
| `LLM_WIKI_DATA_DIR` | `--data-dir` 的回退值 |
| `LLM_WIKI_BIND_HOST` | 绑定地址覆盖（如 `0.0.0.0`） |
| `LLM_WIKI_API_TOKEN` | API 认证令牌（优先于 `app-state.json` 中的配置） |

### 配置

与桌面应用共享同一份 `app-state.json`。主要配置项：

```json
{
  "llmConfig": { "provider": "openai", "apiKey": "sk-xxx", "model": "gpt-4o" },
  "apiConfig": { "enabled": true, "token": "your-token", "allowUnauthenticated": false },
  "embeddingConfig": { "enabled": false }
}
```

如果 `--data-dir` 指向桌面应用的数据目录，则自动继承桌面应用的 LLM 密钥和项目列表。

### API 路由

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/v1/health` | 服务状态（无需认证） |
| GET | `/api/v1/projects` | 项目列表 |
| GET | `/api/v1/projects/{id}/files` | 文件树 |
| GET | `/api/v1/projects/{id}/files/content` | 读取文件内容 |
| GET | `/api/v1/projects/{id}/reviews` | 审核项列表 |
| POST | `/api/v1/projects/{id}/reviews/resolve` | 批量解决审核 |
| PATCH | `/api/v1/projects/{id}/reviews/{id}` | 更新单个审核 |
| POST | `/api/v1/projects/{id}/search` | 关键词+向量混合搜索 |
| GET | `/api/v1/projects/{id}/graph` | 知识图谱数据 |
| POST | `/api/v1/projects/{id}/chat` | RAG 聊天（支持 SSE 流式） |
| POST | `/api/v1/projects/{id}/sources/rescan` | 重新扫描源文件 |

### 注意事项

- **项目列表**：从 `app-state.json` 的 `projectRegistry` 和 `recentProjects` 读取。独立运行时不会像桌面应用那样通过 Chrome 扩展自动更新项目列表。如需要，重启进程以刷新。
- **LLM 配置**：支持与桌面应用相同的所有 provider（OpenAI、Anthropic、Google、Azure、Ollama、Custom、MiniMax），不支持 Claude Code CLI 和 Codex CLI（子进程模式不可用）。
- **macOS 签名**：独立二进制未签名，首次运行可能需要右键 → 打开。

### 未移植部分

| 功能 | 原因 |
|------|------|
| Chrome 扩展通信（Clip Server） | 无桌面环境，不需要 |
| 系统托盘 | 纯服务进程 |
| 全局 HTTP 代理 | 使用系统代理即可 |
| GUI 设置界面 | 通过直接编辑 `app-state.json` 配置 |
| Source rescan（文件监视） | 返回 501；用 `llm-wiki-ingest` 替代 |

---

## 二、llm-wiki-ingest — 独立 Wiki 构建工具

### 简介

命令行批量构建工具：扫描项目的 `raw/sources/` 目录，通过两步 LLM 管道（分析 → 生成）自动创建和更新 wiki 页面。适合：

- 在新机器上重建 wiki
- 添加大量新文档后批量处理
- 定期定时任务自动更新 wiki

### 编译

```bash
cargo build --release --bin llm-wiki-ingest
```

产物位于 `target/release/llm-wiki-ingest`。

### 运行

```bash
# 扫描整个 raw/sources/ 目录
llm-wiki-ingest --project ~/Documents/my-wiki

# 处理单个文件
llm-wiki-ingest --project ~/Documents/my-wiki \
  --file raw/sources/my-paper.pdf

# 强制重建（忽略 SHA-256 缓存）
llm-wiki-ingest --project . --force

# 预览模式（不调用 LLM，仅列出待处理文件）
llm-wiki-ingest --project . --dry-run

# 详细模式（打印 prompt 和 LLM 响应）
llm-wiki-ingest --project . --verbose

# 指定文件夹分类提示
llm-wiki-ingest --project . \
  --folder-context "papers/energy"
```

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `--project` | `.` | Wiki 项目目录（包含 `purpose.md`、`raw/sources/`、`wiki/`） |
| `--data-dir` | 平台默认 | LLM 配置目录 |
| `--file` | 无 | 处理单个文件（相对于 `--project` 的路径） |
| `--force` | false | 无视缓存强制重建 |
| `--dry-run` | false | 仅扫描不调用 LLM |
| `--verbose` / `-v` | false | 打印 prompt 和 LLM 输出 |
| `--folder-context` | 无 | 传递给 LLM 的分类提示 |

### 工作流程

```
对每个 raw/sources/ 下的文件：

1. 计算 SHA-256 → 如果内容未变且非 --force，跳过
2. 读取源文件内容（PDF/DOCX/PPTX/XLSX 自动解析为文本）
3. 加载 purpose.md、schema.md、wiki/index.md、wiki/overview.md
4. 步骤 1 — 分析：LLM 读取源文件，输出结构化分析
   （实体、概念、论点、与现有 wiki 的关联、矛盾和建议）
5. 步骤 2 — 生成：LLM 获取分析结果，输出 FILE 块
   （源摘要页面 + 实体页面 + 概念页面 + index/overview/log 更新）
6. 解析 FILE 块 → 写入 wiki/ 目录
7. 保存 SHA-256 缓存

输出示例：
  [1/5] raw/sources/intro.pdf  (125 KB)
    → 3 pages  (18.7s)
    wrote: wiki/sources/intro.md
    wrote: wiki/entities/transformer.md
    wrote: wiki/concepts/attention.md

  [2/5] raw/sources/notes.md  (8 KB)
    cache hit → skipped  (0.1s)
```

### 注意事项

- **首次使用前需要配置 LLM**：确保 `app-state.json` 中有 `llmConfig`。可以在桌面应用中先配置好，或手动创建 `app-state.json`。
- **LLM 消耗**：两步管道会产生两次 LLM 调用（分析 + 生成）。每个文件的费用取决于源文件大小和使用的模型。
- **文件格式**：支持 PDF（内置 pdf-extract）、DOCX、PPTX、XLSX、ODS。纯文本/Markdown 文件直接读取。
- **缓存**：缓存在 `.llm-wiki/ingest-cache.json`。如果源文件内容未变，重复运行会跳过。用 `--force` 强制重建。
- **并发**：串行处理（一次一个文件），避免 LLM API 速率限制。

### 未移植部分

以下功能存在于桌面应用的 ingest 管道中，但独立 CLI 未实现：

| 功能 | 原因 | 影响 |
|------|------|------|
| **MinerU 云解析** | 需要额外 API 配置和异步轮询 | 复杂 PDF（表格、公式）用内置 pdf-extract 解析，效果可能不如 MinerU |
| **长文本分块** | 代码量大（~300 行 TS），大多数文档不超阈值 | 超大文件（> 80K 字符）可能超出 LLM 上下文窗口 |
| **图片提取 + Vision 标注** | 依赖 pdfium 二进制和多模态 LLM | PDF 中的嵌入图片不会被提取和标注 |
| **第三步 Review 建议** | 额外 LLM 调用，批量场景性价比低 | 不会生成缺失页面、重复检测、研究建议等审核项 |
| **持久化队列** | 批量模式不需要断点续传 | 进程中断后需从头开始（但缓存会跳过已处理的文件） |

---

## 三、平台默认数据目录

| 平台 | 路径 |
|------|------|
| macOS | `~/Library/Application Support/com.llm-wiki.app/` |
| Linux | `~/.local/share/llm-wiki/` |
| Windows | `%APPDATA%\llm-wiki\` |

两个工具都通过 `--data-dir` 或 `LLM_WIKI_DATA_DIR` 环境变量覆盖此路径。

---

## 四、app-state.json 最小配置

```json
{
  "llmConfig": {
    "provider": "custom",
    "apiKey": "sk-your-api-key",
    "model": "deepseek-chat",
    "customEndpoint": "https://api.deepseek.com/v1",
    "maxContextSize": 128000
  },
  "apiConfig": {
    "enabled": true,
    "token": "your-api-token"
  }
}
```

`provider` 支持的值：`openai`、`anthropic`、`google`、`azure`、`ollama`、`custom`、`minimax`。

`custom` 时可通过 `apiMode` 切换协议：`"chat_completions"`（默认，OpenAI 兼容）或 `"anthropic_messages"`（Anthropic 兼容）。
