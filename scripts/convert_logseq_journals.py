#!/usr/bin/env python3
"""Convert logseq_journals to /YYYY/MM/YYYY-MM-DD nesting."""

from __future__ import annotations

import argparse
import os
import re
from typing import Any, Dict, List, Tuple

import httpx

DEFAULT_ENDPOINT = "http://127.0.0.1:6806"
DEFAULT_NOTEBOOK_NAME = "logseq-import"
ROOT_HPATH = "/logseq_journals"

DATE_RE = re.compile(r"^(?P<y>\d{4})_(?P<m>\d{2})_(?P<d>\d{2})$")


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
        # Use a stable placeholder so dry-run can continue showing planned moves.
        return f"dry-run:{hpath}"
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


def parse_date_title(title: str) -> Tuple[str, str, str] | None:
    match = DATE_RE.fullmatch(title)
    if not match:
        return None
    return match.group("y"), match.group("m"), match.group("d")


def main() -> int:
    parser = argparse.ArgumentParser(description="Convert logseq_journals to YYYY/MM/YYYY-MM-DD.")
    parser.add_argument("--endpoint", default=os.getenv("SIYUAN_ENDPOINT", DEFAULT_ENDPOINT))
    parser.add_argument("--token", default=os.getenv("SIYUAN_TOKEN"))
    parser.add_argument("--notebook", default=DEFAULT_NOTEBOOK_NAME)
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--batch-size", type=int, default=200)
    args = parser.parse_args()

    headers = {}
    if args.token:
        headers["Authorization"] = f"Token {args.token}"

    with httpx.Client(headers=headers, timeout=30.0) as client:
        notebook_id = get_notebook_id(client, args.endpoint, args.notebook)

        processed = 0
        skipped = 0
        skipped_existing = 0
        skipped_invalid = 0

        while True:
            stmt = (
                "select id, content, hpath from blocks "
                "where type='d' and box='{}' "
                "and hpath like '/logseq_journals/%' "
                "and content glob '????_??_??' "
                "order by updated desc "
                "limit {};"
            ).format(notebook_id, max(1, args.batch_size))
            docs = sql_query(client, args.endpoint, stmt)
            if not docs:
                break

            print(f"found {len(docs)} date docs under {ROOT_HPATH}")
            processed_batch = 0

            for doc in docs:
                doc_id = doc.get("id")
                title = (doc.get("content") or "").strip()
                hpath = doc.get("hpath") or ""
                if not doc_id or not title:
                    skipped_invalid += 1
                    continue

                date_parts = parse_date_title(title)
                if not date_parts:
                    skipped_invalid += 1
                    continue

                year, month, day = date_parts
                target_parent = f"{ROOT_HPATH}/{year}/{month}"
                target_title = f"{year}-{month}-{day}"
                target_hpath = f"{target_parent}/{target_title}"

                # Skip if already in the expected target location.
                if hpath == target_hpath:
                    skipped += 1
                    continue

                # Avoid collisions if the target doc already exists.
                existing_ids = get_ids_by_hpath(client, args.endpoint, notebook_id, target_hpath)
                if existing_ids and doc_id not in existing_ids:
                    print(f"[warn] target exists, skip {hpath} -> {target_hpath}")
                    skipped_existing += 1
                    continue

                year_id = ensure_doc_by_hpath(
                    client,
                    args.endpoint,
                    notebook_id,
                    f"{ROOT_HPATH}/{year}",
                    year,
                    args.dry_run,
                )
                month_id = ensure_doc_by_hpath(
                    client,
                    args.endpoint,
                    notebook_id,
                    target_parent,
                    month,
                    args.dry_run,
                )
                if args.dry_run and not month_id:
                    print(f"[dry-run] parent id unknown, skip move for {hpath}")
                    skipped += 1
                    continue

                move_doc_by_id(client, args.endpoint, doc_id, month_id, args.dry_run)
                rename_doc_by_id(client, args.endpoint, doc_id, target_title, args.dry_run)
                processed += 1
                processed_batch += 1

            if processed_batch == 0:
                print("[warn] no docs processed in this batch, stop to avoid infinite loop")
                break

        skipped_total = skipped + skipped_existing + skipped_invalid
        print(
            "processed {}, skipped {} (existing {}, invalid {})".format(
                processed,
                skipped_total,
                skipped_existing,
                skipped_invalid,
            )
        )

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
