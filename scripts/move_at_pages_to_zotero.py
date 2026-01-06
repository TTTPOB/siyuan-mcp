#!/usr/bin/env python3
"""Move /pages/@* docs into /zotero articles in logseq-import."""

from __future__ import annotations

import argparse
import os
from typing import Any, Dict, List

import httpx

DEFAULT_ENDPOINT = "http://127.0.0.1:6806"
DEFAULT_NOTEBOOK_NAME = "logseq-import"
TARGET_HPATH = "/zotero articles"


def api_post(client: httpx.Client, endpoint: str, path: str, payload: Dict[str, Any] | None) -> Any:
    url = endpoint.rstrip("/") + path
    resp = client.post(url, json=payload or {})
    resp.raise_for_status()
    data = resp.json()
    if data.get("code") != 0:
        raise RuntimeError(f"API error {path}: {data.get('msg')}")
    return data.get("data")


def get_notebook_id(client: httpx.Client, endpoint: str, name: str) -> str:
    data = api_post(client, endpoint, "/api/notebook/lsNotebooks", None)
    for nb in data.get("notebooks", []):
        if nb.get("name") == name:
            return nb["id"]
    raise RuntimeError(f"Notebook not found: {name}")


def sql_query(client: httpx.Client, endpoint: str, stmt: str) -> List[Dict[str, Any]]:
    data = api_post(client, endpoint, "/api/query/sql", {"stmt": stmt})
    return data or []


def get_ids_by_hpath(client: httpx.Client, endpoint: str, notebook: str, hpath: str) -> List[str]:
    data = api_post(client, endpoint, "/api/filetree/getIDsByHPath", {"path": hpath, "notebook": notebook})
    return data or []


def ensure_doc_by_hpath(
    client: httpx.Client,
    endpoint: str,
    notebook: str,
    hpath: str,
    title: str,
    dry_run: bool,
) -> str:
    ids = get_ids_by_hpath(client, endpoint, notebook, hpath)
    if ids:
        if len(ids) > 1:
            print(f"[warn] multiple docs for {hpath}, use {ids[0]}")
        return ids[0]
    if dry_run:
        print(f"[dry-run] create doc: {hpath}")
        return ""
    md = f"# {title}\n"
    doc_id = api_post(
        client,
        endpoint,
        "/api/filetree/createDocWithMd",
        {"notebook": notebook, "path": hpath, "markdown": md},
    )
    return doc_id


def move_doc_by_id(client: httpx.Client, endpoint: str, doc_id: str, to_id: str, dry_run: bool) -> None:
    if dry_run:
        print(f"[dry-run] move {doc_id} -> {to_id}")
        return
    api_post(client, endpoint, "/api/filetree/moveDocsByID", {"fromIDs": [doc_id], "toID": to_id})


def main() -> int:
    parser = argparse.ArgumentParser(description="Move /pages/@* docs into /zotero articles.")
    parser.add_argument("--endpoint", default=os.getenv("SIYUAN_ENDPOINT", DEFAULT_ENDPOINT))
    parser.add_argument("--token", default=os.getenv("SIYUAN_TOKEN"))
    parser.add_argument("--notebook", default=DEFAULT_NOTEBOOK_NAME)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    headers = {}
    if args.token:
        headers["Authorization"] = f"Token {args.token}"

    with httpx.Client(headers=headers, timeout=30.0) as client:
        notebook_id = get_notebook_id(client, args.endpoint, args.notebook)

        stmt = (
            "select id, content, hpath from blocks "
            "where type='d' and box='{}' and hpath like '/pages/@%' "
            "order by updated desc;"
        ).format(notebook_id)
        docs = sql_query(client, args.endpoint, stmt)
        print(f"found {len(docs)} '@' docs in {args.notebook}")

        target_id = ensure_doc_by_hpath(
            client,
            args.endpoint,
            notebook_id,
            TARGET_HPATH,
            "zotero articles",
            args.dry_run,
        )
        if not target_id and args.dry_run:
            print("[dry-run] target id unknown, moves skipped")
            return 0

        for doc in docs:
            doc_id = doc.get("id")
            if not doc_id:
                continue
            move_doc_by_id(client, args.endpoint, doc_id, target_id, args.dry_run)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
