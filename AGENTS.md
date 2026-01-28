# AGENTS.md instructions for /home/tpob/playground/siyuan-mcp

<INSTRUCTIONS>
User requires all responses to be in Chinese, regardless of the user's language. Code comments must remain in English.
toolchain:
    if created a new node project, use pnpm instead of npm or yarn
    if created a new python project, use uv instead of pip venv

    if modifing a exisitng project, keep original toolchain unless user specified.

--- project-doc ---

## SiYuan Note System - Critical Notes

### Working with SiYuan API

#### Document Move Operations - CRITICAL
When moving documents (e.g. via `/api/file/moveDocs` or path-based move APIs), SiYuan expects **storage paths**, not human-readable names.

Key rules:
- Storage paths use **document IDs** and usually end with **`.sy`** (e.g. `/notebookID/parentDocID/targetDocID.sy`).
- Human-readable paths (hpath) like `/daily note/2025/09/2025-09-08` are **NOT** valid for move APIs that operate on storage paths.
- If you need a target folder path, first resolve it by ID using `/api/file/getPathByID` (MCP: `siyuan_note_siyuan_doc_get_path_by_id`).

Preferred approach:
- Use ID-based moves whenever possible (MCP: `siyuan_note_siyuan_doc_move_by_id`) to avoid constructing storage paths.

Example:
```python
# Prefer moving by IDs (no manual .sy paths)
api_post("/api/file/moveDocs", {
    "fromIDs": [src_doc_id],
    "toID": target_parent_doc_id,
})

# If you must move by paths, use storage paths (IDs + .sy)
target_parent_path = api_post("/api/file/getPathByID", {"id": target_parent_doc_id})
to_path = f"{target_parent_path}/{src_doc_id}.sy"
```

#### Block Update Operations - CRITICAL
When updating SiYuan blocks via `/api/block/updateBlock`, you MUST preserve the kramdown format including IAL (Inline Attribute List) metadata.

**Wrong approach (will corrupt blocks):**
```python
# DO NOT do this - using SQL content field
content = sql_query("SELECT content FROM blocks WHERE id='...'")
api_post("/api/block/updateBlock", {
    "id": block_id,
    "dataType": "markdown", 
    "data": content  # Missing IAL metadata!
})
```

**Correct approach:**
```python
# ALWAYS get kramdown first
kramdown = api_post("/api/block/getBlockKramdown", {"id": block_id})
# kramdown includes IAL: "content here\n{: id=\"...\" updated=\"...\"}"

# Modify the kramdown
new_kramdown = modify_content(kramdown)

# Update with kramdown
api_post("/api/block/updateBlock", {
    "id": block_id,
    "dataType": "markdown",
    "data": new_kramdown  # IAL preserved!
})
```

**Why this matters:**
- SiYuan blocks store metadata in IAL format: `{: id="block-id" updated="timestamp"}`
- SQL `content` field is plain text without IAL
- SQL `markdown` field has IAL but may be stale
- `/api/block/getBlockKramdown` always returns current kramdown with IAL
- Updating without IAL destroys block structure and causes data corruption

**Consequences of wrong approach:**
- Page structure collapses (lists become paragraphs, nesting lost)
- Block metadata corrupted
- User must restore from file history
- Data loss risk

#### Date Reference Conversion

When converting Logseq-style date references to SiYuan block references:

1. **Always create journal documents first** before converting references
2. **Support multiple date formats:**
   - `[[YYYY-MM-DD]]` → ISO format (e.g., `[[2025-01-26]]`)
   - `[[Month DDth, YYYY]]` → English format (e.g., `[[Sep 8th, 2025]]`)
3. **Convert to SiYuan block reference:** `((block-id "YYYY-MM-DD"))`
4. **Journal document structure:** `/journals/YYYY/MM/YYYY-MM-DD`
5. **Always test on a single block first** before mass conversion
6. **Always backup before batch operations**

**Example workflow:**
```python
# 1. Collect all referenced dates
dates = extract_dates_from_content(all_blocks)

# 2. Create missing journal docs
for date in dates:
    if not journal_exists(date):
        create_journal_doc(notebook_id, f"/journals/{year}/{month}/{date}")

# 3. Build date-to-block-id mapping
date_map = build_date_map_from_journals(notebook_id)

# 4. Replace references (using kramdown!)
for block in blocks_with_dates:
    kramdown = get_block_kramdown(block_id)
    new_kramdown = replace_dates(kramdown, date_map)
    update_block(block_id, new_kramdown)
```

**Scripts available:**
- `scripts/replace_lilab_idp_dates.py` - Convert `[[YYYY-MM-DD]]` format
- `scripts/replace_month_dates.py` - Convert `[[Month DDth, YYYY]]` format
- Both support `--dry-run` flag for testing

### Best Practices

1. **Always use dry-run first:** Test with `--dry-run` flag before actual operations
2. **Backup critical data:** Export or copy content before batch operations
3. **Test on single block:** Validate the approach on one block before scaling
4. **Verify after update:** Check that block structure is preserved
5. **Use kramdown API:** Never rely on SQL `content` field for updates
6. **Preserve IAL:** IAL metadata is essential for SiYuan's internal consistency

## Skills
These skills are discovered at startup from multiple local sources. Each entry includes a name, description, and file path so you can open the source for full instructions.
- beads: Use when a task requires interacting with the beads/bd issue tracker (create/list/update/close/search issues, manage dependencies/epics/duplicates, sync/export/import data, or inspect beads status) in a repository that uses beads. (file: /home/tpob/.codex/skills/beads/SKILL.md)
- skill-creator: Guide for creating effective skills. This skill should be used when users want to create a new skill (or update an existing skill) that extends Codex's capabilities with specialized knowledge, workflows, or tool integrations. (file: /home/tpob/.codex/skills/.system/skill-creator/SKILL.md)
- skill-installer: Install Codex skills into $CODEX_HOME/skills from a curated list or a GitHub repo path. Use when a user asks to list installable skills, install a curated skill, or install a skill from another repo (including private repos). (file: /home/tpob/.codex/skills/.system/skill-installer/SKILL.md)
- Discovery: Available skills are listed in project docs and may also appear in a runtime "## Skills" section (name + description + file path). These are the sources of truth; skill bodies live on disk at the listed paths.
- Trigger rules: If the user names a skill (with `$SkillName` or plain text) OR the task clearly matches a skill's description, you must use that skill for that turn. Multiple mentions mean use them all. Do not carry skills across turns unless re-mentioned.
- Missing/blocked: If a named skill isn't in the list or the path can't be read, say so briefly and continue with the best fallback.
- How to use a skill (progressive disclosure):
  1) After deciding to use a skill, open its `SKILL.md`. Read only enough to follow the workflow.
  2) If `SKILL.md` points to extra folders such as `references/`, load only the specific files needed for the request; don't bulk-load everything.
  3) If `scripts/` exist, prefer running or patching them instead of retyping large code blocks.
  4) If `assets/` or templates exist, reuse them instead of recreating from scratch.
- Description as trigger: The YAML `description` in `SKILL.md` is the primary trigger signal; rely on it to decide applicability. If unsure, ask a brief clarification before proceeding.
- Coordination and sequencing:
  - If multiple skills apply, choose the minimal set that covers the request and state the order you'll use them.
  - Announce which skill(s) you're using and why (one short line). If you skip an obvious skill, say why.
- Context hygiene:
  - Keep context small: summarize long sections instead of pasting them; only load extra files when needed.
  - Avoid deeply nested references; prefer one-hop files explicitly linked from `SKILL.md`.
  - When variants exist (frameworks, providers, domains), pick only the relevant reference file(s) and note that choice.
- Safety and fallback: If a skill can't be applied cleanly (missing files, unclear instructions), state the issue, pick the next-best approach, and continue.
</INSTRUCTIONS>
