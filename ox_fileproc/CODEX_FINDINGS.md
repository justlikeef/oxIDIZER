I reviewed the core loader + cursor/scanner paths. Here are the key findings, ordered by severity with file/line refs.

**Findings**
- **High – untrusted config can read arbitrary files by default.** `process_file` builds a `Processor` with `root_dir: None`, so `include` can read absolute paths or traverse outside the config directory. If configs are user‑controlled, this is a serious data‑exposure risk; strongly consider requiring `with_root_dir(...)` in public APIs or providing a “safe” constructor that enforces a root. `src/processor.rs:330-333`, `src/processor.rs:228-301`
- **Medium – JSON key search can match string values in arrays as if they were keys.** `JsonScanner::find_child` only tracks `{}` depth (not `[]`) and doesn’t verify a `:` after the matched string. A string value inside an array at top object depth can be misidentified as a key, yielding empty/incorrect spans and wrong edits. `src/scanners/json.rs:15-116`
- **Medium – directory includes can traverse symlinked dirs outside root before blocking.** Root checks are done on the initial include path, but recursive directory traversal (`read_dir`/`is_dir`) will follow symlinked subdirs and read their entries before `load_recursive` enforces root on files. That still leaks directory enumeration outside root. Consider canonicalizing and validating each directory before `read_dir`/`is_dir`. `src/processor.rs:245-259`
- **Low – recursion depth is off by one.** The guard triggers on `current_depth > max_depth`, so a `max_depth` of 5 allows depth 6. If you intend a strict limit, change to `>=`. `src/processor.rs:81-83`
- **Low – YAML scanner truncates quoted values containing `#` in single‑quoted scalars.** The inline value parser treats `#` as comment unless inside quotes, but the quote‑tracking logic assumes backslash escaping for both quote types. YAML single quotes escape via `''`, so `'#'` or `it''s # ok` can be truncated. `src/scanners/yaml.rs:53-73`
- **Low – numeric ID merge uses `as_f64` which can miscompare large integers.** Large IDs beyond 53 bits will lose precision and may merge incorrectly. Consider integer‑safe comparisons or stringifying numbers for matching. `src/processor.rs:424-427`

**Open questions / assumptions**
- Is `process_file` intended for trusted inputs only? If not, I’d treat the default “no root restriction” as a security vulnerability rather than an API choice.
- Do you want YAML/JSON scanners to be “best effort” or correctness‑critical? If correctness matters, these should likely be backed by parsers rather than line/byte heuristics.

I didn’t make any code changes.
