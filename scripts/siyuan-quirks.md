# SiYuan quirks and findings (from script development)

## 1) SQL API result cap / implicit pagination
- Symptom: `/api/query/sql` appeared to return exactly 64 rows per call even without `LIMIT`.
  - Example: `convert_logseq_journals.py` repeatedly found `64` date docs and only finished after multiple runs.
- Impact: Any logic that expects a full result set in one call will silently miss rows.
- Mitigation:
  - Always page results explicitly (use `LIMIT` + `OFFSET` or keyset pagination).
  - For large scans, prefer keyset pagination (`id > last_id ORDER BY id`) to avoid offset drift when updating blocks.

## 2) Dry-run behavior can hide parent creation
- Symptom: In dry-run mode, missing parent docs (e.g., `/logseq_journals/2023/06`) were not created, so moves were skipped with “parent id unknown”.
- Impact: Dry-run output looked incomplete even though the logic was correct for real runs.
- Mitigation:
  - In dry-run, return a stable placeholder ID for newly created parents so the script can still show planned move/rename operations.

## 3) `hpath` filtering only catches document blocks
- Symptom: Querying `blocks` with `hpath like '/pages/%'` returns *document* blocks only, not child blocks.
  - Example: Page `freeipa下的用户创建` contains `[[2025-09-16]]`, but the link blocks are children; the filter missed them.
- Impact: Scripts that search content by `hpath` alone will miss links inside pages.
- Mitigation:
  - Filter by `root_id` in a subquery: select blocks where `root_id` is a `/pages/` document.

## 4) Block updates can fail with “block not found” mid-run
- Symptom: `/api/block/updateBlock` returned `get block failed: block not found` during a replace run.
- Impact: Entire script aborted even though only one block disappeared.
- Mitigation:
  - Catch this error, log a warning, and continue.
  - Combine with keyset pagination to reduce skipping/duplication when the dataset changes during updates.

## 5) Title-based parsing is brittle
- Symptom: Journal conversion relies on `content` matching `YYYY_MM_DD` or `YYYY-MM-DD`.
- Impact: Non-standard titles (or duplicates) are skipped, and duplicates can cause ambiguity.
- Mitigation:
  - Validate titles strictly; warn on duplicates; avoid overwriting existing targets.

## 6) Notebook lookup
- Note: There is no `boxes` table exposed to SQL (attempts returned “no such table: boxes”).
- Impact: Notebook ID must be fetched via `/api/notebook/lsNotebooks` and used as `box` in SQL.

## 7) Moving vs. renaming order
- Observation: Moving a doc then renaming is stable for journal conversion (parent IDs resolved by `getIDsByHPath`).
- Advice: Always ensure parent docs exist before moving, then rename to avoid path collisions.

## 8) Journal map load also needs pagination
- Symptom: Initial journal map had only 64 entries; replacements didn’t happen even though pages contained `[[YYYY-MM-DD]]`.
- Impact: Replacement failed silently for dates outside the first batch.
- Mitigation:
  - Paginate the journal doc query until exhausted.

