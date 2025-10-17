## `apply_patch`

Use the `apply_patch` shell command to edit files.
Your patch language is a stripped‑down, file‑oriented diff format designed to be easy to parse and safe to apply. You can think of it as a high‑level envelope:

*** Begin Patch
[ one or more file sections ]
*** End Patch

Within that envelope, you get a sequence of file operations.
You MUST include a header to specify the action you are taking.
Each operation starts with one of three headers:

*** Add File: <path> - create a new file. Every following line is a + line (the initial contents).
*** Delete File: <path> - remove an existing file. Nothing follows.
*** Update File: <path> - patch an existing file in place (optionally with a rename).

May be immediately followed by *** Move to: <new path> if you want to rename the file.
Then one or more “hunks”, each introduced by @@ (optionally followed by a hunk header).
Within a hunk each line starts with:

For instructions on [context_before] and [context_after]:
- By default, show 3 lines of code immediately above and 3 lines immediately below each change. If a change is within 3 lines of a previous change, do NOT duplicate the first change’s [context_after] lines in the second change’s [context_before] lines.
- If 3 lines of context is insufficient to uniquely identify the snippet of code within the file, use the @@ operator to indicate the class or function to which the snippet belongs. For instance, we might have:
@@ class BaseClass
[3 lines of pre-context]
- [old_code]
+ [new_code]
[3 lines of post-context]

- If a code block is repeated so many times in a class or function such that even a single `@@` statement and 3 lines of context cannot uniquely identify the snippet of code, you can use multiple `@@` statements to jump to the right context. For instance:

@@ class BaseClass
@@ 	 def method():
[3 lines of pre-context]
- [old_code]
+ [new_code]
[3 lines of post-context]

The full grammar definition is below:
Patch := Begin { FileOp } End
Begin := "*** Begin Patch" NEWLINE
End := "*** End Patch" NEWLINE
FileOp := AddFile | DeleteFile | UpdateFile
AddFile := "*** Add File: " path NEWLINE { "+" line NEWLINE }
DeleteFile := "*** Delete File: " path NEWLINE
UpdateFile := "*** Update File: " path NEWLINE [ MoveTo ] { Hunk }
MoveTo := "*** Move to: " newPath NEWLINE
Hunk := "@@" [ header ] NEWLINE { HunkLine } [ "*** End of File" NEWLINE ]
HunkLine := (" " | "-" | "+") text NEWLINE

A full patch can combine several operations:

*** Begin Patch
*** Add File: hello.txt
+Hello world
*** Update File: src/app.py
*** Move to: src/main.py
@@ def greet():
-print("Hi")
+print("Hello, world!")
*** Delete File: obsolete.txt
*** End Patch

It is important to remember:

- You must include a header with your intended action (Add/Delete/Update)
- You must prefix new lines with `+` even when creating a new file
- File references can only be relative, NEVER ABSOLUTE.

You can invoke apply_patch like:

```
shell {"command":["apply_patch","*** Begin Patch\n*** Add File: hello.txt\n+Hello, world!\n*** End Patch\n"]}
```

### Output

On success the tool prints a begin_patch-style summary to stdout so you always know what happened without re-reading the files:

```
Applied operations:
- add: hello.txt (+1)
- update: src/main.rs (+3, -1)
✔ Patch applied successfully.
```

Each bullet lists the action (`add`, `update`, `move`, or `delete`) plus the per-file line deltas.

Move operations appear as `- move: source -> dest (+added, -removed)` and combine renames with content edits in a single entry. Deletes show their line count (`- delete: path (-N)`). When a patch touches multiple files the summary lists one bullet per file in patch order so you can skim the outcome at a glance. All filesystem updates are applied atomically: every file is written through a temporary file and the original contents are backed up, so a failure automatically rolls the workspace back to its pre-patch state.

If patch verification fails, `apply_patch` prints the diagnostic to stderr and leaves the filesystem unchanged.

On success, any touched files are automatically staged in git (when run inside a repository), so your workspace is ready for a commit without additional `git add` commands.
### CLI options

The `apply_patch` binary accepts several quality-of-life flags:

- `-f/--patch-file <path>` – load the patch from disk instead of argv/STDIN.
- `-C/--root <dir>` – execute all file operations relative to the provided root.
- `--dry-run` – plan the operations without writing to disk (the summary shows `planned` statuses).
- `--output-format {human|json|both}` – control whether to emit JSON, the human summary, or both (`human` by default).
- `--json-path <path>` – write the JSON report to a file regardless of the chosen output format.
- `--no-summary` – suppress the human summary when `human` output is enabled.
- `--machine` – emit a single-line JSON object following the `apply_patch/v2` schema (no human summary, ignores `--output-format`).
- `--log-dir <path>` – write structured JSON logs to the given directory (default: `reports/logs`).
- `--log-retention-days <days>` – prune log files older than the given number of days (default: 14).
- `--log-keep <count>` – keep at most this many log files after pruning (default: 200).
- `--no-logs` – skip writing per-run logs.
- `--conflict-dir <path>` – write conflict diff hints to this directory when a patch fails (default: `reports/conflicts`).

The JSON report mirrors begin_patch’s schema (`status`, `mode`, `duration_ms`, per-file operations, and `errors`).
