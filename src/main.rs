use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use clap::Parser;
use pmcp::{Error, RequestHandlerExtra, Server, ToolHandler};
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
}

#[derive(Clone, Copy)]
enum ToolKind {
    Json,
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
}

#[async_trait]
impl ToolHandler for SiyuanTool {
    async fn handle(&self, args: Value, _extra: RequestHandlerExtra) -> Result<Value, Error> {
        match self.kind {
            ToolKind::Json => {
                let body = Self::ensure_object(args)?;
                self.client.post_json_value(self.endpoint, body).await
            }
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
