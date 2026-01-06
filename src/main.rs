use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use clap::Parser;
use pmcp::{Error, RequestHandlerExtra, Server, ToolHandler};
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

    async fn post_json_value(&self, endpoint: &str, body: Value) -> Result<Value, Error> {
        let url = format!("{}{}", self.base_url, endpoint);
        let mut req = self.client.post(url).json(&body);
        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Token {}", token));
        }
        let resp = req
            .send()
            .await
            .map_err(|err| Error::internal(err.to_string()))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|err| Error::internal(err.to_string()))?;
        match serde_json::from_str::<Value>(&text) {
            Ok(value) => Ok(value),
            Err(_) => Ok(json!({ "status": status.as_u16(), "body": text })),
        }
    }

    async fn post_multipart_value(&self, endpoint: &str, form: Form) -> Result<Value, Error> {
        let url = format!("{}{}", self.base_url, endpoint);
        let mut req = self.client.post(url).multipart(form);
        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Token {}", token));
        }
        let resp = req
            .send()
            .await
            .map_err(|err| Error::internal(err.to_string()))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|err| Error::internal(err.to_string()))?;
        match serde_json::from_str::<Value>(&text) {
            Ok(value) => Ok(value),
            Err(_) => Ok(json!({ "status": status.as_u16(), "body": text })),
        }
    }

    async fn post_json_file(&self, endpoint: &str, body: Value) -> Result<Value, Error> {
        let url = format!("{}{}", self.base_url, endpoint);
        let mut req = self.client.post(url).json(&body);
        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Token {}", token));
        }
        let resp = req
            .send()
            .await
            .map_err(|err| Error::internal(err.to_string()))?;
        let status = resp.status();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.to_string());
        let bytes = resp
            .bytes()
            .await
            .map_err(|err| Error::internal(err.to_string()))?;
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
    fn new(client: Arc<SiyuanClient>, endpoint: &'static str, kind: ToolKind) -> Self {
        Self {
            client,
            endpoint,
            kind,
        }
    }

    fn ensure_object(args: Value) -> Result<Value, Error> {
        match args {
            Value::Object(_) => Ok(args),
            Value::Null => Ok(json!({})),
            _ => Err(Error::validation("arguments must be a JSON object")),
        }
    }

    fn args_as_object(args: Value) -> Result<serde_json::Map<String, Value>, Error> {
        match args {
            Value::Object(map) => Ok(map),
            Value::Null => Ok(serde_json::Map::new()),
            _ => Err(Error::validation("arguments must be a JSON object")),
        }
    }

    fn required_string(map: &serde_json::Map<String, Value>, key: &str) -> Result<String, Error> {
        map.get(key)
            .and_then(|value| value.as_str())
            .map(|value| value.to_string())
            .ok_or_else(|| Error::validation(format!("missing or invalid `{}`", key)))
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

    fn string_array(map: &serde_json::Map<String, Value>, key: &str) -> Result<Vec<String>, Error> {
        let values = map
            .get(key)
            .and_then(|value| value.as_array())
            .ok_or_else(|| Error::validation(format!("missing or invalid `{}`", key)))?;
        let mut out = Vec::with_capacity(values.len());
        for value in values {
            let item = value
                .as_str()
                .ok_or_else(|| Error::validation(format!("invalid `{}` entry", key)))?;
            out.push(item.to_string());
        }
        Ok(out)
    }

    async fn file_part(file_path: &str) -> Result<Part, Error> {
        let bytes = tokio::fs::read(file_path)
            .await
            .map_err(|err| Error::internal(format!("read file {}: {}", file_path, err)))?;
        let filename = Path::new(file_path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file")
            .to_string();
        Ok(Part::bytes(bytes).file_name(filename))
    }

    async fn handle_asset_upload(&self, args: Value) -> Result<Value, Error> {
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

    async fn handle_put_file(&self, args: Value) -> Result<Value, Error> {
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

    async fn handle_get_file(&self, args: Value) -> Result<Value, Error> {
        let map = Self::args_as_object(args)?;
        let path = Self::required_string(&map, "path")?;
        let body = json!({ "path": path });
        self.client.post_json_file(self.endpoint, body).await
    }
}

#[async_trait]
impl ToolHandler for SiyuanTool {
    async fn handle(&self, args: Value, _extra: RequestHandlerExtra) -> Result<Value, Error> {
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
}

const TOOL_SPECS: &[ToolSpec] = &[
    ToolSpec {
        name: "siyuan_notebook_ls",
        endpoint: "/api/notebook/lsNotebooks",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_notebook_open",
        endpoint: "/api/notebook/openNotebook",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_notebook_close",
        endpoint: "/api/notebook/closeNotebook",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_notebook_rename",
        endpoint: "/api/notebook/renameNotebook",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_notebook_create",
        endpoint: "/api/notebook/createNotebook",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_notebook_remove",
        endpoint: "/api/notebook/removeNotebook",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_notebook_get_conf",
        endpoint: "/api/notebook/getNotebookConf",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_notebook_set_conf",
        endpoint: "/api/notebook/setNotebookConf",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_doc_create_md",
        endpoint: "/api/filetree/createDocWithMd",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_doc_rename",
        endpoint: "/api/filetree/renameDoc",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_doc_rename_by_id",
        endpoint: "/api/filetree/renameDocByID",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_doc_remove",
        endpoint: "/api/filetree/removeDoc",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_doc_remove_by_id",
        endpoint: "/api/filetree/removeDocByID",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_doc_move",
        endpoint: "/api/filetree/moveDocs",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_doc_move_by_id",
        endpoint: "/api/filetree/moveDocsByID",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_doc_get_hpath_by_path",
        endpoint: "/api/filetree/getHPathByPath",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_doc_get_hpath_by_id",
        endpoint: "/api/filetree/getHPathByID",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_doc_get_path_by_id",
        endpoint: "/api/filetree/getPathByID",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_doc_get_ids_by_hpath",
        endpoint: "/api/filetree/getIDsByHPath",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_asset_upload",
        endpoint: "/api/asset/upload",
        kind: ToolKind::AssetUpload,
    },
    ToolSpec {
        name: "siyuan_block_insert",
        endpoint: "/api/block/insertBlock",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_block_prepend",
        endpoint: "/api/block/prependBlock",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_block_append",
        endpoint: "/api/block/appendBlock",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_block_update",
        endpoint: "/api/block/updateBlock",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_block_delete",
        endpoint: "/api/block/deleteBlock",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_block_move",
        endpoint: "/api/block/moveBlock",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_block_fold",
        endpoint: "/api/block/foldBlock",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_block_unfold",
        endpoint: "/api/block/unfoldBlock",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_block_get_kramdown",
        endpoint: "/api/block/getBlockKramdown",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_block_get_children",
        endpoint: "/api/block/getChildBlocks",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_block_transfer_ref",
        endpoint: "/api/block/transferBlockRef",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_attr_set",
        endpoint: "/api/attr/setBlockAttrs",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_attr_get",
        endpoint: "/api/attr/getBlockAttrs",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_sql_query",
        endpoint: "/api/query/sql",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_sql_flush",
        endpoint: "/api/sqlite/flushTransaction",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_template_render",
        endpoint: "/api/template/render",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_template_render_sprig",
        endpoint: "/api/template/renderSprig",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_file_get",
        endpoint: "/api/file/getFile",
        kind: ToolKind::GetFile,
    },
    ToolSpec {
        name: "siyuan_file_put",
        endpoint: "/api/file/putFile",
        kind: ToolKind::PutFile,
    },
    ToolSpec {
        name: "siyuan_file_remove",
        endpoint: "/api/file/removeFile",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_file_rename",
        endpoint: "/api/file/renameFile",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_file_read_dir",
        endpoint: "/api/file/readDir",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_export_md",
        endpoint: "/api/export/exportMdContent",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_export_resources",
        endpoint: "/api/export/exportResources",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_convert_pandoc",
        endpoint: "/api/convert/pandoc",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_notify_msg",
        endpoint: "/api/notification/pushMsg",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_notify_err",
        endpoint: "/api/notification/pushErrMsg",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_network_forward_proxy",
        endpoint: "/api/network/forwardProxy",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_system_boot_progress",
        endpoint: "/api/system/bootProgress",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_system_version",
        endpoint: "/api/system/version",
        kind: ToolKind::Json,
    },
    ToolSpec {
        name: "siyuan_system_current_time",
        endpoint: "/api/system/currentTime",
        kind: ToolKind::Json,
    },
];

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let client = Arc::new(SiyuanClient::new(
        args.base_url,
        args.token,
        args.timeout_ms,
    )?);

    let mut builder = Server::builder()
        .name("siyuan-mcp")
        .version(env!("CARGO_PKG_VERSION"));

    for spec in TOOL_SPECS {
        let tool = SiyuanTool::new(client.clone(), spec.endpoint, spec.kind);
        builder = builder.tool(spec.name, tool);
    }

    let server = builder.build()?;
    server.run_stdio().await?;

    Ok(())
}
