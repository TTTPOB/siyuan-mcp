use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use base64::{engine::general_purpose, Engine as _};
use clap::Parser;
use log::{debug, info};
use rmcp::{
    ErrorData as McpError,
    RoleServer,
    ServerHandler,
    ServiceExt,
    model::{
        CallToolRequestParam, CallToolResult, Content, Implementation, JsonObject,
        ListToolsResult, PaginatedRequestParam, ServerCapabilities, ServerInfo, Tool,
    },
    service::RequestContext,
    transport::stdio,
};
use reqwest::multipart::{Form, Part};
use serde_json::{json, Value};

#[derive(Debug, Parser)]
#[command(name = "siyuan-mcp", version, about = "SiYuan MCP server")]
struct Args {
    #[arg(long, env = "SIYUAN_BASE_URL", default_value = "http://127.0.0.1:6806")]
    base_url: String,
    #[arg(long, env = "SIYUAN_TOKEN")]
    token: Option<String>,
    #[arg(long, env = "SIYUAN_TIMEOUT_MS", default_value_t = 15000)]
    timeout_ms: u64,
}

#[derive(Clone)]
struct SiyuanClient {
    base_url: String,
    token: Option<String>,
    client: reqwest::Client,
}

impl SiyuanClient {
    fn new(base_url: String, token: Option<String>, timeout_ms: u64) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .context("build reqwest client")?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            token,
            client,
        })
    }

    async fn post_json_value(&self, endpoint: &str, body: Value) -> Result<Value, McpError> {
        let url = format!("{}{}", self.base_url, endpoint);
        let mut req = self.client.post(url).json(&body);
        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Token {}", token));
        }
        let resp = req
            .send()
            .await
            .map_err(|err| McpError::internal_error(err.to_string(), None))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|err| McpError::internal_error(err.to_string(), None))?;
        match serde_json::from_str::<Value>(&text) {
            Ok(value) => Ok(value),
            Err(_) => Ok(json!({ "status": status.as_u16(), "body": text })),
        }
    }

    async fn post_multipart_value(&self, endpoint: &str, form: Form) -> Result<Value, McpError> {
        let url = format!("{}{}", self.base_url, endpoint);
        let mut req = self.client.post(url).multipart(form);
        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Token {}", token));
        }
        let resp = req
            .send()
            .await
            .map_err(|err| McpError::internal_error(err.to_string(), None))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|err| McpError::internal_error(err.to_string(), None))?;
        match serde_json::from_str::<Value>(&text) {
            Ok(value) => Ok(value),
            Err(_) => Ok(json!({ "status": status.as_u16(), "body": text })),
        }
    }

    async fn post_json_file(&self, endpoint: &str, body: Value) -> Result<Value, McpError> {
        let url = format!("{}{}", self.base_url, endpoint);
        let mut req = self.client.post(url).json(&body);
        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Token {}", token));
        }
        let resp = req
            .send()
            .await
            .map_err(|err| McpError::internal_error(err.to_string(), None))?;
        let status = resp.status();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.to_string());
        let bytes = resp
            .bytes()
            .await
            .map_err(|err| McpError::internal_error(err.to_string(), None))?;
        if status.is_success() {
            let encoded = general_purpose::STANDARD.encode(&bytes);
            Ok(json!({
                "status": status.as_u16(),
                "content_type": content_type,
                "body_base64": encoded
            }))
        } else if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
            Ok(value)
        } else {
            let encoded = general_purpose::STANDARD.encode(&bytes);
            Ok(json!({
                "status": status.as_u16(),
                "content_type": content_type,
                "body_base64": encoded
            }))
        }
    }
}

#[derive(Clone, Copy)]
enum ToolKind {
    Json,
    AssetUpload,
    PutFile,
    GetFile,
}

#[derive(Clone)]
struct SiyuanTool {
    client: Arc<SiyuanClient>,
    endpoint: &'static str,
    kind: ToolKind,
}

impl SiyuanTool {
    fn new(client: Arc<SiyuanClient>, spec: &ToolSpec) -> Self {
        Self {
            client,
            endpoint: spec.endpoint,
            kind: spec.kind,
        }
    }

    fn ensure_object(args: Value) -> Result<Value, McpError> {
        match args {
            Value::Object(_) => Ok(args),
            Value::Null => Ok(json!({})),
            _ => Err(McpError::invalid_params(
                "arguments must be a JSON object",
                None,
            )),
        }
    }

    fn args_as_object(args: Value) -> Result<serde_json::Map<String, Value>, McpError> {
        match args {
            Value::Object(map) => Ok(map),
            Value::Null => Ok(serde_json::Map::new()),
            _ => Err(McpError::invalid_params(
                "arguments must be a JSON object",
                None,
            )),
        }
    }

    fn required_string(
        map: &serde_json::Map<String, Value>,
        key: &str,
    ) -> Result<String, McpError> {
        map.get(key)
            .and_then(|value| value.as_str())
            .map(|value| value.to_string())
            .ok_or_else(|| {
                McpError::invalid_params(format!("missing or invalid `{}`", key), None)
            })
    }

    fn optional_string(map: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
        map.get(key)
            .and_then(|value| value.as_str())
            .map(|value| value.to_string())
    }

    fn optional_bool(map: &serde_json::Map<String, Value>, key: &str) -> Option<bool> {
        map.get(key).and_then(|value| value.as_bool())
    }

    fn optional_u64(map: &serde_json::Map<String, Value>, key: &str) -> Option<u64> {
        map.get(key).and_then(|value| value.as_u64())
    }

    fn string_array(
        map: &serde_json::Map<String, Value>,
        key: &str,
    ) -> Result<Vec<String>, McpError> {
        let values = map
            .get(key)
            .and_then(|value| value.as_array())
            .ok_or_else(|| McpError::invalid_params(format!("missing or invalid `{}`", key), None))?;
        let mut out = Vec::with_capacity(values.len());
        for value in values {
            let item = value
                .as_str()
                .ok_or_else(|| McpError::invalid_params(format!("invalid `{}` entry", key), None))?;
            out.push(item.to_string());
        }
        Ok(out)
    }

    async fn file_part(file_path: &str) -> Result<Part, McpError> {
        let bytes = tokio::fs::read(file_path)
            .await
            .map_err(|err| {
                McpError::internal_error(format!("read file {}: {}", file_path, err), None)
            })?;
        let filename = Path::new(file_path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file")
            .to_string();
        Ok(Part::bytes(bytes).file_name(filename))
    }

    async fn handle_asset_upload(&self, args: Value) -> Result<Value, McpError> {
        let map = Self::args_as_object(args)?;
        let assets_dir =
            Self::optional_string(&map, "assets_dir_path").unwrap_or_else(|| "/assets/".to_string());
        let files = Self::string_array(&map, "files")?;
        let mut form = Form::new().text("assetsDirPath", assets_dir);
        for file_path in files {
            let part = Self::file_part(&file_path).await?;
            form = form.part("file[]", part);
        }
        self.client.post_multipart_value(self.endpoint, form).await
    }

    async fn handle_put_file(&self, args: Value) -> Result<Value, McpError> {
        let map = Self::args_as_object(args)?;
        let path = Self::required_string(&map, "path")?;
        let is_dir = Self::optional_bool(&map, "is_dir");
        let mod_time = Self::optional_u64(&map, "mod_time");
        let mut form = Form::new().text("path", path);
        if let Some(value) = is_dir {
            form = form.text("isDir", value.to_string());
        }
        if let Some(value) = mod_time {
            form = form.text("modTime", value.to_string());
        }
        let is_dir_flag = is_dir.unwrap_or(false);
        if !is_dir_flag {
            let file_path = Self::required_string(&map, "file_path")?;
            let part = Self::file_part(&file_path).await?;
            form = form.part("file", part);
        }
        self.client.post_multipart_value(self.endpoint, form).await
    }

    async fn handle_get_file(&self, args: Value) -> Result<Value, McpError> {
        let map = Self::args_as_object(args)?;
        let path = Self::required_string(&map, "path")?;
        let body = json!({ "path": path });
        self.client.post_json_file(self.endpoint, body).await
    }
    async fn handle(&self, args: Value) -> Result<Value, McpError> {
        match self.kind {
            ToolKind::Json => {
                let body = Self::ensure_object(args)?;
                self.client.post_json_value(self.endpoint, body).await
            }
            ToolKind::AssetUpload => self.handle_asset_upload(args).await,
            ToolKind::PutFile => self.handle_put_file(args).await,
            ToolKind::GetFile => self.handle_get_file(args).await,
        }
    }
}

struct ToolSpec {
    name: &'static str,
    endpoint: &'static str,
    kind: ToolKind,
    description: &'static str,
    schema: &'static str,
}

fn parse_schema(schema: &'static str) -> JsonObject {
    match serde_json::from_str::<Value>(schema) {
        Ok(Value::Object(map)) => map,
        _ => JsonObject::default(),
    }
}

const SCHEMA_EMPTY: &str =
    r#"{"type":"object","properties":{},"additionalProperties":false}"#;
const SCHEMA_NOTEBOOK_ID: &str = r#"{"type":"object","properties":{"notebook":{"type":"string","description":"Notebook ID"}},"required":["notebook"],"additionalProperties":true}"#;
const SCHEMA_NOTEBOOK_ID_NAME: &str = r#"{"type":"object","properties":{"notebook":{"type":"string","description":"Notebook ID"},"name":{"type":"string","description":"Notebook name"}},"required":["notebook","name"],"additionalProperties":true}"#;
const SCHEMA_NOTEBOOK_CREATE: &str = r#"{"type":"object","properties":{"name":{"type":"string","description":"Notebook name"}},"required":["name"],"additionalProperties":true}"#;
const SCHEMA_NOTEBOOK_CONF: &str = r#"{"type":"object","properties":{"notebook":{"type":"string","description":"Notebook ID"},"conf":{"type":"object","description":"Notebook config object"}},"required":["notebook","conf"],"additionalProperties":true}"#;
const SCHEMA_DOC_CREATE_MD: &str = r#"{"type":"object","properties":{"notebook":{"type":"string","description":"Notebook ID"},"path":{"type":"string","description":"Document path (hpath)"},"markdown":{"type":"string","description":"GFM markdown content"}},"required":["notebook","path","markdown"],"additionalProperties":true}"#;
const SCHEMA_DOC_RENAME: &str = r#"{"type":"object","properties":{"notebook":{"type":"string","description":"Notebook ID"},"path":{"type":"string","description":"Document path"},"title":{"type":"string","description":"New document title"}},"required":["notebook","path","title"],"additionalProperties":true}"#;
const SCHEMA_DOC_RENAME_BY_ID: &str = r#"{"type":"object","properties":{"id":{"type":"string","description":"Document ID"},"title":{"type":"string","description":"New document title"}},"required":["id","title"],"additionalProperties":true}"#;
const SCHEMA_DOC_REMOVE: &str = r#"{"type":"object","properties":{"notebook":{"type":"string","description":"Notebook ID"},"path":{"type":"string","description":"Document path"}},"required":["notebook","path"],"additionalProperties":true}"#;
const SCHEMA_ID_ONLY: &str = r#"{"type":"object","properties":{"id":{"type":"string","description":"Block or document ID"}},"required":["id"],"additionalProperties":true}"#;
const SCHEMA_DOC_MOVE: &str = r#"{"type":"object","properties":{"fromPaths":{"type":"array","items":{"type":"string"},"description":"Source document paths"},"toNotebook":{"type":"string","description":"Target notebook ID"},"toPath":{"type":"string","description":"Target path"}},"required":["fromPaths","toNotebook","toPath"],"additionalProperties":true}"#;
const SCHEMA_DOC_MOVE_BY_ID: &str = r#"{"type":"object","properties":{"fromIDs":{"type":"array","items":{"type":"string"},"description":"Source document IDs"},"toID":{"type":"string","description":"Target parent doc ID or notebook ID"}},"required":["fromIDs","toID"],"additionalProperties":true}"#;
const SCHEMA_GET_HPATH_BY_PATH: &str = r#"{"type":"object","properties":{"notebook":{"type":"string","description":"Notebook ID"},"path":{"type":"string","description":"Document path"}},"required":["notebook","path"],"additionalProperties":true}"#;
const SCHEMA_GET_IDS_BY_HPATH: &str = r#"{"type":"object","properties":{"path":{"type":"string","description":"Human-readable path"},"notebook":{"type":"string","description":"Notebook ID"}},"required":["path","notebook"],"additionalProperties":true}"#;
const SCHEMA_BLOCK_INSERT: &str = r#"{"type":"object","properties":{"dataType":{"type":"string","description":"markdown or dom"},"data":{"type":"string","description":"Content to insert"},"nextID":{"type":"string","description":"Next block ID"},"previousID":{"type":"string","description":"Previous block ID"},"parentID":{"type":"string","description":"Parent block ID"}},"required":["dataType","data"],"additionalProperties":true}"#;
const SCHEMA_BLOCK_PREPEND: &str = r#"{"type":"object","properties":{"dataType":{"type":"string","description":"markdown or dom"},"data":{"type":"string","description":"Content to insert"},"parentID":{"type":"string","description":"Parent block ID"}},"required":["dataType","data","parentID"],"additionalProperties":true}"#;
const SCHEMA_BLOCK_UPDATE: &str = r#"{"type":"object","properties":{"dataType":{"type":"string","description":"markdown or dom"},"data":{"type":"string","description":"Updated content"},"id":{"type":"string","description":"Block ID"}},"required":["dataType","data","id"],"additionalProperties":true}"#;
const SCHEMA_BLOCK_MOVE: &str = r#"{"type":"object","properties":{"id":{"type":"string","description":"Block ID"},"previousID":{"type":"string","description":"Previous block ID"},"parentID":{"type":"string","description":"Parent block ID"}},"required":["id"],"additionalProperties":true}"#;
const SCHEMA_BLOCK_TRANSFER_REF: &str = r#"{"type":"object","properties":{"fromID":{"type":"string","description":"Def block ID"},"toID":{"type":"string","description":"Target block ID"},"refIDs":{"type":"array","items":{"type":"string"},"description":"Optional ref block IDs"}},"required":["fromID","toID"],"additionalProperties":true}"#;
const SCHEMA_ATTR_SET: &str = r#"{"type":"object","properties":{"id":{"type":"string","description":"Block ID"},"attrs":{"type":"object","description":"Attributes map"}},"required":["id","attrs"],"additionalProperties":true}"#;
const SCHEMA_SQL_QUERY: &str = r#"{"type":"object","properties":{"stmt":{"type":"string","description":"SQL statement"}},"required":["stmt"],"additionalProperties":true}"#;
const SCHEMA_TEMPLATE_RENDER: &str = r#"{"type":"object","properties":{"id":{"type":"string","description":"Document ID"},"path":{"type":"string","description":"Template file absolute path"}},"required":["id","path"],"additionalProperties":true}"#;
const SCHEMA_TEMPLATE_RENDER_SPRIG: &str = r#"{"type":"object","properties":{"template":{"type":"string","description":"Template content"}},"required":["template"],"additionalProperties":true}"#;
const SCHEMA_FILE_PATH: &str = r#"{"type":"object","properties":{"path":{"type":"string","description":"Path under workspace"}},"required":["path"],"additionalProperties":true}"#;
const SCHEMA_FILE_PUT: &str = r#"{"type":"object","properties":{"path":{"type":"string","description":"Path under workspace"},"is_dir":{"type":"boolean","description":"Create directory only"},"mod_time":{"type":"integer","description":"Unix time (seconds)"},"file_path":{"type":"string","description":"Local file path to upload"}},"required":["path"],"additionalProperties":true}"#;
const SCHEMA_FILE_RENAME: &str = r#"{"type":"object","properties":{"path":{"type":"string","description":"Path under workspace"},"newPath":{"type":"string","description":"New path under workspace"}},"required":["path","newPath"],"additionalProperties":true}"#;
const SCHEMA_FILE_READ_DIR: &str = r#"{"type":"object","properties":{"path":{"type":"string","description":"Directory path under workspace"}},"required":["path"],"additionalProperties":true}"#;
const SCHEMA_EXPORT_MD: &str = r#"{"type":"object","properties":{"id":{"type":"string","description":"Doc block ID"}},"required":["id"],"additionalProperties":true}"#;
const SCHEMA_EXPORT_RESOURCES: &str = r#"{"type":"object","properties":{"paths":{"type":"array","items":{"type":"string"},"description":"Paths to export"},"name":{"type":"string","description":"Optional zip name"}},"required":["paths"],"additionalProperties":true}"#;
const SCHEMA_PANDOC: &str = r#"{"type":"object","properties":{"dir":{"type":"string","description":"Working directory name"},"args":{"type":"array","items":{"type":"string"},"description":"Pandoc CLI args"}},"required":["dir","args"],"additionalProperties":true}"#;
const SCHEMA_NOTIFY: &str = r#"{"type":"object","properties":{"msg":{"type":"string","description":"Message text"},"timeout":{"type":"integer","description":"Timeout in ms"}},"required":["msg"],"additionalProperties":true}"#;
const SCHEMA_NETWORK_FORWARD_PROXY: &str = r#"{"type":"object","properties":{"url":{"type":"string","description":"Target URL"},"method":{"type":"string","description":"HTTP method"},"timeout":{"type":"integer","description":"Timeout in ms"},"contentType":{"type":"string","description":"Content-Type"},"headers":{"type":"array","items":{"type":"object"},"description":"Headers list"},"payload":{"type":"object","description":"Payload object or string"},"payloadEncoding":{"type":"string","description":"Payload encoding"},"responseEncoding":{"type":"string","description":"Response body encoding"}},"required":["url"],"additionalProperties":true}"#;
const SCHEMA_ASSET_UPLOAD: &str = r#"{"type":"object","properties":{"assets_dir_path":{"type":"string","description":"Target assets dir (e.g. /assets/)"},"files":{"type":"array","items":{"type":"string"},"description":"Local file paths"}},"required":["files"],"additionalProperties":true}"#;

const TOOL_SPECS: &[ToolSpec] = &[
    ToolSpec {
        name: "siyuan_notebook_ls",
        endpoint: "/api/notebook/lsNotebooks",
        kind: ToolKind::Json,
        description: "List notebooks. No parameters. Use to obtain notebook IDs.",
        schema: SCHEMA_EMPTY,
    },
    ToolSpec {
        name: "siyuan_notebook_open",
        endpoint: "/api/notebook/openNotebook",
        kind: ToolKind::Json,
        description: "Open a notebook by ID.",
        schema: SCHEMA_NOTEBOOK_ID,
    },
    ToolSpec {
        name: "siyuan_notebook_close",
        endpoint: "/api/notebook/closeNotebook",
        kind: ToolKind::Json,
        description: "Close a notebook by ID.",
        schema: SCHEMA_NOTEBOOK_ID,
    },
    ToolSpec {
        name: "siyuan_notebook_rename",
        endpoint: "/api/notebook/renameNotebook",
        kind: ToolKind::Json,
        description: "Rename a notebook by ID.",
        schema: SCHEMA_NOTEBOOK_ID_NAME,
    },
    ToolSpec {
        name: "siyuan_notebook_create",
        endpoint: "/api/notebook/createNotebook",
        kind: ToolKind::Json,
        description: "Create a new notebook.",
        schema: SCHEMA_NOTEBOOK_CREATE,
    },
    ToolSpec {
        name: "siyuan_notebook_remove",
        endpoint: "/api/notebook/removeNotebook",
        kind: ToolKind::Json,
        description: "Remove a notebook by ID.",
        schema: SCHEMA_NOTEBOOK_ID,
    },
    ToolSpec {
        name: "siyuan_notebook_get_conf",
        endpoint: "/api/notebook/getNotebookConf",
        kind: ToolKind::Json,
        description: "Fetch notebook configuration by ID.",
        schema: SCHEMA_NOTEBOOK_ID,
    },
    ToolSpec {
        name: "siyuan_notebook_set_conf",
        endpoint: "/api/notebook/setNotebookConf",
        kind: ToolKind::Json,
        description: "Save notebook configuration by ID.",
        schema: SCHEMA_NOTEBOOK_CONF,
    },
    ToolSpec {
        name: "siyuan_doc_create_md",
        endpoint: "/api/filetree/createDocWithMd",
        kind: ToolKind::Json,
        description: "Create a document with Markdown content.",
        schema: SCHEMA_DOC_CREATE_MD,
    },
    ToolSpec {
        name: "siyuan_doc_rename",
        endpoint: "/api/filetree/renameDoc",
        kind: ToolKind::Json,
        description: "Rename a document by notebook + path.",
        schema: SCHEMA_DOC_RENAME,
    },
    ToolSpec {
        name: "siyuan_doc_rename_by_id",
        endpoint: "/api/filetree/renameDocByID",
        kind: ToolKind::Json,
        description: "Rename a document by ID.",
        schema: SCHEMA_DOC_RENAME_BY_ID,
    },
    ToolSpec {
        name: "siyuan_doc_remove",
        endpoint: "/api/filetree/removeDoc",
        kind: ToolKind::Json,
        description: "Remove a document by notebook + path.",
        schema: SCHEMA_DOC_REMOVE,
    },
    ToolSpec {
        name: "siyuan_doc_remove_by_id",
        endpoint: "/api/filetree/removeDocByID",
        kind: ToolKind::Json,
        description: "Remove a document by ID.",
        schema: SCHEMA_ID_ONLY,
    },
    ToolSpec {
        name: "siyuan_doc_move",
        endpoint: "/api/filetree/moveDocs",
        kind: ToolKind::Json,
        description: "Move documents by source paths to a target notebook/path.",
        schema: SCHEMA_DOC_MOVE,
    },
    ToolSpec {
        name: "siyuan_doc_move_by_id",
        endpoint: "/api/filetree/moveDocsByID",
        kind: ToolKind::Json,
        description: "Move documents by IDs to a target parent ID or notebook ID.",
        schema: SCHEMA_DOC_MOVE_BY_ID,
    },
    ToolSpec {
        name: "siyuan_doc_get_hpath_by_path",
        endpoint: "/api/filetree/getHPathByPath",
        kind: ToolKind::Json,
        description: "Get human-readable path from notebook + storage path.",
        schema: SCHEMA_GET_HPATH_BY_PATH,
    },
    ToolSpec {
        name: "siyuan_doc_get_hpath_by_id",
        endpoint: "/api/filetree/getHPathByID",
        kind: ToolKind::Json,
        description: "Get human-readable path from block/document ID.",
        schema: SCHEMA_ID_ONLY,
    },
    ToolSpec {
        name: "siyuan_doc_get_path_by_id",
        endpoint: "/api/filetree/getPathByID",
        kind: ToolKind::Json,
        description: "Get storage path and notebook ID from block/document ID.",
        schema: SCHEMA_ID_ONLY,
    },
    ToolSpec {
        name: "siyuan_doc_get_ids_by_hpath",
        endpoint: "/api/filetree/getIDsByHPath",
        kind: ToolKind::Json,
        description: "Get IDs from human-readable path + notebook ID.",
        schema: SCHEMA_GET_IDS_BY_HPATH,
    },
    ToolSpec {
        name: "siyuan_asset_upload",
        endpoint: "/api/asset/upload",
        kind: ToolKind::AssetUpload,
        description: "Upload assets from local files. Uses multipart. Params: assets_dir_path, files[].",
        schema: SCHEMA_ASSET_UPLOAD,
    },
    ToolSpec {
        name: "siyuan_block_insert",
        endpoint: "/api/block/insertBlock",
        kind: ToolKind::Json,
        description: "Insert blocks using nextID/previousID/parentID anchors.",
        schema: SCHEMA_BLOCK_INSERT,
    },
    ToolSpec {
        name: "siyuan_block_prepend",
        endpoint: "/api/block/prependBlock",
        kind: ToolKind::Json,
        description: "Prepend blocks to parentID.",
        schema: SCHEMA_BLOCK_PREPEND,
    },
    ToolSpec {
        name: "siyuan_block_append",
        endpoint: "/api/block/appendBlock",
        kind: ToolKind::Json,
        description: "Append blocks to parentID.",
        schema: SCHEMA_BLOCK_PREPEND,
    },
    ToolSpec {
        name: "siyuan_block_update",
        endpoint: "/api/block/updateBlock",
        kind: ToolKind::Json,
        description: "Update a block by ID.",
        schema: SCHEMA_BLOCK_UPDATE,
    },
    ToolSpec {
        name: "siyuan_block_delete",
        endpoint: "/api/block/deleteBlock",
        kind: ToolKind::Json,
        description: "Delete a block by ID.",
        schema: SCHEMA_ID_ONLY,
    },
    ToolSpec {
        name: "siyuan_block_move",
        endpoint: "/api/block/moveBlock",
        kind: ToolKind::Json,
        description: "Move a block with previousID/parentID anchors.",
        schema: SCHEMA_BLOCK_MOVE,
    },
    ToolSpec {
        name: "siyuan_block_fold",
        endpoint: "/api/block/foldBlock",
        kind: ToolKind::Json,
        description: "Fold a block by ID.",
        schema: SCHEMA_ID_ONLY,
    },
    ToolSpec {
        name: "siyuan_block_unfold",
        endpoint: "/api/block/unfoldBlock",
        kind: ToolKind::Json,
        description: "Unfold a block by ID.",
        schema: SCHEMA_ID_ONLY,
    },
    ToolSpec {
        name: "siyuan_block_get_kramdown",
        endpoint: "/api/block/getBlockKramdown",
        kind: ToolKind::Json,
        description: "Get block kramdown by ID.",
        schema: SCHEMA_ID_ONLY,
    },
    ToolSpec {
        name: "siyuan_block_get_children",
        endpoint: "/api/block/getChildBlocks",
        kind: ToolKind::Json,
        description: "List child blocks by parent ID.",
        schema: SCHEMA_ID_ONLY,
    },
    ToolSpec {
        name: "siyuan_block_transfer_ref",
        endpoint: "/api/block/transferBlockRef",
        kind: ToolKind::Json,
        description: "Transfer block references from one def block to another.",
        schema: SCHEMA_BLOCK_TRANSFER_REF,
    },
    ToolSpec {
        name: "siyuan_attr_set",
        endpoint: "/api/attr/setBlockAttrs",
        kind: ToolKind::Json,
        description: "Set block attributes.",
        schema: SCHEMA_ATTR_SET,
    },
    ToolSpec {
        name: "siyuan_attr_get",
        endpoint: "/api/attr/getBlockAttrs",
        kind: ToolKind::Json,
        description: "Get block attributes by ID.",
        schema: SCHEMA_ID_ONLY,
    },
    ToolSpec {
        name: "siyuan_sql_query",
        endpoint: "/api/query/sql",
        kind: ToolKind::Json,
        description: "Execute SQL query against SiYuan database.",
        schema: SCHEMA_SQL_QUERY,
    },
    ToolSpec {
        name: "siyuan_sql_flush",
        endpoint: "/api/sqlite/flushTransaction",
        kind: ToolKind::Json,
        description: "Flush the current SQLite transaction. No parameters.",
        schema: SCHEMA_EMPTY,
    },
    ToolSpec {
        name: "siyuan_template_render",
        endpoint: "/api/template/render",
        kind: ToolKind::Json,
        description: "Render a template file for a document.",
        schema: SCHEMA_TEMPLATE_RENDER,
    },
    ToolSpec {
        name: "siyuan_template_render_sprig",
        endpoint: "/api/template/renderSprig",
        kind: ToolKind::Json,
        description: "Render a Sprig template string.",
        schema: SCHEMA_TEMPLATE_RENDER_SPRIG,
    },
    ToolSpec {
        name: "siyuan_file_get",
        endpoint: "/api/file/getFile",
        kind: ToolKind::GetFile,
        description: "Download a file. Returns body_base64 + content_type.",
        schema: SCHEMA_FILE_PATH,
    },
    ToolSpec {
        name: "siyuan_file_put",
        endpoint: "/api/file/putFile",
        kind: ToolKind::PutFile,
        description: "Upload a file or create a directory (multipart). Params: path, is_dir, mod_time, file_path.",
        schema: SCHEMA_FILE_PUT,
    },
    ToolSpec {
        name: "siyuan_file_remove",
        endpoint: "/api/file/removeFile",
        kind: ToolKind::Json,
        description: "Remove a file by workspace path.",
        schema: SCHEMA_FILE_PATH,
    },
    ToolSpec {
        name: "siyuan_file_rename",
        endpoint: "/api/file/renameFile",
        kind: ToolKind::Json,
        description: "Rename a file by workspace path.",
        schema: SCHEMA_FILE_RENAME,
    },
    ToolSpec {
        name: "siyuan_file_read_dir",
        endpoint: "/api/file/readDir",
        kind: ToolKind::Json,
        description: "List files in a directory by workspace path.",
        schema: SCHEMA_FILE_READ_DIR,
    },
    ToolSpec {
        name: "siyuan_export_md",
        endpoint: "/api/export/exportMdContent",
        kind: ToolKind::Json,
        description: "Export a document as Markdown content by ID.",
        schema: SCHEMA_EXPORT_MD,
    },
    ToolSpec {
        name: "siyuan_export_resources",
        endpoint: "/api/export/exportResources",
        kind: ToolKind::Json,
        description: "Export files/folders to a zip; returns zip path.",
        schema: SCHEMA_EXPORT_RESOURCES,
    },
    ToolSpec {
        name: "siyuan_convert_pandoc",
        endpoint: "/api/convert/pandoc",
        kind: ToolKind::Json,
        description: "Run pandoc conversion in a temp directory.",
        schema: SCHEMA_PANDOC,
    },
    ToolSpec {
        name: "siyuan_notify_msg",
        endpoint: "/api/notification/pushMsg",
        kind: ToolKind::Json,
        description: "Push a normal notification message.",
        schema: SCHEMA_NOTIFY,
    },
    ToolSpec {
        name: "siyuan_notify_err",
        endpoint: "/api/notification/pushErrMsg",
        kind: ToolKind::Json,
        description: "Push an error notification message.",
        schema: SCHEMA_NOTIFY,
    },
    ToolSpec {
        name: "siyuan_network_forward_proxy",
        endpoint: "/api/network/forwardProxy",
        kind: ToolKind::Json,
        description: "Forward proxy HTTP request through SiYuan.",
        schema: SCHEMA_NETWORK_FORWARD_PROXY,
    },
    ToolSpec {
        name: "siyuan_system_boot_progress",
        endpoint: "/api/system/bootProgress",
        kind: ToolKind::Json,
        description: "Get system boot progress. No parameters.",
        schema: SCHEMA_EMPTY,
    },
    ToolSpec {
        name: "siyuan_system_version",
        endpoint: "/api/system/version",
        kind: ToolKind::Json,
        description: "Get system version. No parameters.",
        schema: SCHEMA_EMPTY,
    },
    ToolSpec {
        name: "siyuan_system_current_time",
        endpoint: "/api/system/currentTime",
        kind: ToolKind::Json,
        description: "Get system current time (ms). No parameters.",
        schema: SCHEMA_EMPTY,
    },
];

#[derive(Clone)]
struct SiyuanServer {
    tools: Arc<Vec<Tool>>,
    tool_handlers: Arc<HashMap<&'static str, SiyuanTool>>,
}

impl SiyuanServer {
    fn new(client: Arc<SiyuanClient>) -> Self {
        let mut tools = Vec::new();
        let mut handlers = HashMap::new();
        for spec in TOOL_SPECS {
            let handler = SiyuanTool::new(client.clone(), spec);
            let schema = parse_schema(spec.schema);
            let tool = Tool::new(spec.name, spec.description, Arc::new(schema));
            tools.push(tool);
            handlers.insert(spec.name, handler);
        }
        debug!("registered {} tools", tools.len());
        Self {
            tools: Arc::new(tools),
            tool_handlers: Arc::new(handlers),
        }
    }

    async fn handle_tool_call(&self, name: &str, args: Value) -> Result<Value, McpError> {
        let handler = self.tool_handlers.get(name).ok_or_else(|| {
            McpError::invalid_params(format!("unknown tool: {}", name), None)
        })?;
        handler.handle(args).await
    }
}

impl ServerHandler for SiyuanServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "siyuan-mcp".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        let tools = self.tools.clone();
        async move {
            Ok(ListToolsResult {
                tools: (*tools).clone(),
                next_cursor: None,
            })
        }
    }

    fn call_tool(
        &self,
        request: CallToolRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, McpError>> + Send + '_ {
        let server = self.clone();
        async move {
            let name = request.name.as_ref();
            let args = request
                .arguments
                .map(Value::Object)
                .unwrap_or(Value::Null);
            let result = server.handle_tool_call(name, args).await?;
            let content = Content::json(result)?;
            Ok(CallToolResult::success(vec![content]))
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info"),
    )
    .format_timestamp_millis()
    .init();
    let args = Args::parse();
    info!(
        "starting siyuan-mcp: base_url={}, timeout_ms={}, token_set={}",
        args.base_url,
        args.timeout_ms,
        args.token.is_some()
    );
    let client = Arc::new(SiyuanClient::new(
        args.base_url,
        args.token,
        args.timeout_ms,
    )?);

    let server = SiyuanServer::new(client);
    let running = server.serve(stdio()).await?;
    running.waiting().await?;

    Ok(())
}
