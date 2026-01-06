#!/usr/bin/env python3
"""Convert Logseq ___ separators into nested SiYuan documents."""

from __future__ import annotations

import argparse
import os
from typing import Any, Dict, List

import httpx

DEFAULT_ENDPOINT = "http://127.0.0.1:6806"
DEFAULT_NOTEBOOK_NAME = "logseq-import"


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


def normalize_parent_hpath(hpath: str) -> str:
    if not hpath or hpath == "/":
        return "/"
    if "/" not in hpath[1:]:
        return "/"
    parent = hpath.rsplit("/", 1)[0]
    return parent if parent else "/"


def join_hpath(base: str, parts: List[str]) -> str:
    if not parts:
        return base
    if base == "/":
        return "/" + "/".join(parts)
    return base.rstrip("/") + "/" + "/".join(parts)


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


def rename_doc_by_id(client: httpx.Client, endpoint: str, doc_id: str, title: str, dry_run: bool) -> None:
    if dry_run:
        print(f"[dry-run] rename {doc_id} -> {title}")
        return
    api_post(client, endpoint, "/api/filetree/renameDocByID", {"id": doc_id, "title": title})


def main() -> int:
    parser = argparse.ArgumentParser(description="Convert Logseq ___ separators into nested SiYuan docs.")
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
            "where type='d' and box='{}' and instr(content, '___') > 0 "
            "order by updated desc;"
        ).format(notebook_id)
        docs = sql_query(client, args.endpoint, stmt)

        print(f"found {len(docs)} docs with '___' in {args.notebook}")

        for doc in docs:
            doc_id = doc.get("id")
            content = doc.get("content") or ""
            hpath = doc.get("hpath") or ""

            parts = [p for p in content.split("___") if p != ""]
            if len(parts) < 2:
                print(f"[skip] {doc_id} content='{content}'")
                continue

            parent_hpath = normalize_parent_hpath(hpath)
            new_parent_hpath = join_hpath(parent_hpath, parts[:-1])
            new_title = parts[-1]

            # Ensure parent chain exists
            current_base = parent_hpath
            for part in parts[:-1]:
                current_base = join_hpath(current_base, [part])
                ensure_doc_by_hpath(
                    client,
                    args.endpoint,
                    notebook_id,
                    current_base,
                    part,
                    args.dry_run,
                )

            parent_ids = get_ids_by_hpath(client, args.endpoint, notebook_id, new_parent_hpath)
            if not parent_ids:
                print(f"[skip] parent not found: {new_parent_hpath}")
                continue
            parent_id = parent_ids[0]

            move_doc_by_id(client, args.endpoint, doc_id, parent_id, args.dry_run)
            rename_doc_by_id(client, args.endpoint, doc_id, new_title, args.dry_run)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
