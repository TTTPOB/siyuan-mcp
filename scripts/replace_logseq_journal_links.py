#!/usr/bin/env python3
"""Replace [[YYYY-MM-DD]] with SiYuan block refs in logseq-import/pages."""

from __future__ import annotations

import argparse
import os
import re
from typing import Any, Dict, List, Tuple

import httpx

DEFAULT_ENDPOINT = "http://127.0.0.1:6806"
DEFAULT_SOURCE_NOTEBOOK = "logseq-import"
DEFAULT_JOURNAL_NOTEBOOK = "logseq-journals"

DATE_RE = re.compile(r"\[\[(\d{4}-\d{2}-\d{2})\]\]")


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


def update_block_markdown(
    client: httpx.Client, endpoint: str, block_id: str, markdown: str, dry_run: bool
) -> None:
    if dry_run:
        print(f"[dry-run] update {block_id}")
        return
    try:
        api_post(
            client,
            endpoint,
            "/api/block/updateBlock",
            {"id": block_id, "dataType": "markdown", "data": markdown},
        )
    except RuntimeError as exc:
        # Skip missing blocks to avoid aborting the entire run.
        msg = str(exc)
        if "block not found" in msg:
            print(f"[warn] block not found, skip {block_id}")
            return
        raise


def build_journal_map(
    client: httpx.Client, endpoint: str, notebook_id: str
) -> Dict[str, str]:
    mapping: Dict[str, str] = {}
    offset = 0
    batch_size = 200

    while True:
        stmt = (
            "select id, content from blocks "
            "where type='d' and box='{}' and content glob '????-??-??' "
            "order by id limit {} offset {};"
        ).format(notebook_id, batch_size, offset)
        rows = sql_query(client, endpoint, stmt)
        if not rows:
            break

        for row in rows:
            date = (row.get("content") or "").strip()
            doc_id = row.get("id")
            if not date or not doc_id:
                continue
            if not re.fullmatch(r"\d{4}-\d{2}-\d{2}", date):
                continue
            # If duplicates exist, keep the first and warn.
            if date in mapping and mapping[date] != doc_id:
                print(f"[warn] duplicate date {date}, keep {mapping[date]}")
                continue
            mapping[date] = doc_id

        offset += len(rows)

    return mapping


def fetch_blocks_batch(
    client: httpx.Client,
    endpoint: str,
    notebook_id: str,
    limit: int,
    last_id: str | None,
) -> List[Dict[str, Any]]:
    where_last = ""
    if last_id:
        where_last = f"and id > '{last_id}' "
    stmt = (
        "select id, content, hpath, type from blocks "
        "where box='{}' and content like '%[[%' "
        "and root_id in ("
        "  select id from blocks where type='d' and box='{}' and hpath like '/pages/%'"
        ") "
        "{}"
        "order by id limit {};"
    ).format(notebook_id, notebook_id, where_last, limit)
    return sql_query(client, endpoint, stmt)


def replace_dates_in_text(text: str, mapping: Dict[str, str]) -> Tuple[str, int]:
    replaced = 0

    def _repl(match: re.Match[str]) -> str:
        nonlocal replaced
        date = match.group(1)
        block_id = mapping.get(date)
        if not block_id:
            return match.group(0)
        replaced += 1
        return f'(({block_id} "{date}"))'

    new_text = DATE_RE.sub(_repl, text)
    return new_text, replaced


def main() -> int:
    parser = argparse.ArgumentParser(description="Replace [[YYYY-MM-DD]] with SiYuan refs.")
    parser.add_argument("--endpoint", default=os.getenv("SIYUAN_ENDPOINT", DEFAULT_ENDPOINT))
    parser.add_argument("--token", default=os.getenv("SIYUAN_TOKEN"))
    parser.add_argument("--source-notebook", default=DEFAULT_SOURCE_NOTEBOOK)
    parser.add_argument("--journal-notebook", default=DEFAULT_JOURNAL_NOTEBOOK)
    parser.add_argument("--batch-size", type=int, default=200)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    headers = {}
    if args.token:
        headers["Authorization"] = f"Token {args.token}"

    with httpx.Client(headers=headers, timeout=30.0) as client:
        source_id = get_notebook_id(client, args.endpoint, args.source_notebook)
        journal_id = get_notebook_id(client, args.endpoint, args.journal_notebook)

        journal_map = build_journal_map(client, args.endpoint, journal_id)
        print(f"journal dates loaded: {len(journal_map)}")

        total_blocks = 0
        updated_blocks = 0
        total_replacements = 0
        last_id: str | None = None

        while True:
            rows = fetch_blocks_batch(client, args.endpoint, source_id, args.batch_size, last_id)
            if not rows:
                break
            total_blocks += len(rows)

            for row in rows:
                block_id = row.get("id")
                content = row.get("content") or ""
                if not block_id or not content:
                    continue

                new_content, replaced = replace_dates_in_text(content, journal_map)
                if replaced == 0 or new_content == content:
                    continue

                update_block_markdown(client, args.endpoint, block_id, new_content, args.dry_run)
                updated_blocks += 1
                total_replacements += replaced

            last_id = rows[-1].get("id") or last_id

        print(
            "scanned {} blocks, updated {}, replacements {}".format(
                total_blocks, updated_blocks, total_replacements
            )
        )

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
