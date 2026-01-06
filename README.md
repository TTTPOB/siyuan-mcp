# SiYuan MCP 服务

该项目将 SiYuan 的官方 API 暴露为 MCP 工具，便于在 MCP 生态中调用。

## 运行方式

1. 确保本机已运行 SiYuan（默认地址 `http://127.0.0.1:6806`）。
2. 设置环境变量或使用命令行参数：

```bash
export SIYUAN_TOKEN="你的 Token"
export SIYUAN_BASE_URL="http://127.0.0.1:6806"
export SIYUAN_TIMEOUT_MS=15000
```

3. 启动 MCP 服务（STDIO 模式）：

```bash
cargo run --release
```

或使用 CLI 参数覆盖：

```bash
cargo run --release -- \
  --base-url http://127.0.0.1:6806 \
  --token "你的 Token" \
  --timeout-ms 15000
```

## 配置说明

- `SIYUAN_BASE_URL` / `--base-url`：SiYuan API 基地址（默认 `http://127.0.0.1:6806`）
- `SIYUAN_TOKEN` / `--token`：SiYuan API Token（可在设置-关于中查看）
- `SIYUAN_TIMEOUT_MS` / `--timeout-ms`：请求超时（毫秒）

## 工具选择指南（重点）

所有工具都以 `siyuan_` 前缀命名。大部分工具是“直接转发 JSON 参数”的 API；以下是选择时的重点说明：

### 资产/文件类（multipart 或文件下载）

- `siyuan_asset_upload`
  - 用于上传资源文件到 `/assets/` 或其子目录
  - 参数：`assets_dir_path`（可选，默认 `/assets/`）、`files`（本地文件路径数组）
  - 适用场景：上传图片/附件

- `siyuan_file_put`
  - 用于上传文件或创建目录（multipart）
  - 参数：
    - `path`（必填，工作区内路径）
    - `is_dir`（可选，true 则仅创建目录）
    - `mod_time`（可选，Unix 时间戳秒）
    - `file_path`（当 `is_dir=false` 时必填，本地文件路径）
  - 适用场景：把本地文件写入工作区、批量写入中间文件

- `siyuan_file_get`
  - 用于下载工作区内文件
  - 参数：`path`
  - 返回：`body_base64` + `content_type`（二进制内容以 base64 返回）

### 文档与块

- 文档创建：`siyuan_doc_create_md`
- 文档移动/重命名/删除：`siyuan_doc_move`、`siyuan_doc_rename`、`siyuan_doc_remove` 及 `*_by_id`
- 块操作：`siyuan_block_insert` / `prepend` / `append` / `update` / `delete`

### 查询与导出

- SQL 查询：`siyuan_sql_query`
- 导出 Markdown：`siyuan_export_md`
- 导出文件/目录为 zip：`siyuan_export_resources`

### 系统与通知

- 系统信息：`siyuan_system_version` / `siyuan_system_current_time`
- 通知：`siyuan_notify_msg` / `siyuan_notify_err`

## 返回格式说明

- 大多数 JSON API 会返回标准结构：

```json
{ "code": 0, "msg": "", "data": {} }
```

- 文件下载工具 `siyuan_file_get` 返回：

```json
{ "status": 200, "content_type": "...", "body_base64": "..." }
```

## 备注

- 所有工具的详细输入 schema 已内置在 MCP 工具元数据中，调用方可根据 schema 自动提示。

## 已知怪异行为：SQL 查询疑似隐式分页（约 64 条）

在实际脚本处理中发现：即使 SQL 没写 `LIMIT`，`/api/query/sql` 也可能只返回约 64 条记录（表现为“每次运行只处理 64 条，反复跑几次才全部完成”）。这会导致一次性扫描/替换只覆盖部分数据。

### 示例 1：日志表现

```text
python convert_logseq_journals.py
found 64 date docs under /logseq_journals
processed 64, skipped 0 (existing 0, invalid 0)

python convert_logseq_journals.py
found 64 date docs under /logseq_journals
processed 64, skipped 0 (existing 0, invalid 0)

python convert_logseq_journals.py
found 23 date docs under /logseq_journals
processed 23, skipped 0 (existing 0, invalid 0)
```

### 示例 2：替换脚本的分页修复

初版脚本在构建日期映射时只加载到 64 条，导致替换几乎不生效：

```text
journal dates loaded: 64
scanned 699 blocks, updated 0, replacements 0
```

修复后改为分页加载全部结果：

```text
journal dates loaded: 212
scanned 699 blocks, updated 146, replacements 187
```

### 建议

- 对 SQL 结果做**显式分页**（`LIMIT/OFFSET` 或 keyset 分页）。
- 对“会更新数据”的扫描，优先用 keyset 分页避免 offset 漂移。
